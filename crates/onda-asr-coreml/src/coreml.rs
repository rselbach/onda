//! Thin wrapper over CoreML models and multi-arrays. This is the only module
//! in the crate allowed to use `unsafe`.
//!
//! Every CoreML call runs inside [`guarded`]: an Objective-C exception
//! unwinding into Rust aborts the process, so it becomes an error instead.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::ptr::NonNull;

use block2::StackBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_ml::{
    MLComputeUnits, MLDictionaryFeatureProvider, MLFeatureProvider, MLFeatureValue, MLModel,
    MLModelConfiguration, MLMultiArray, MLMultiArrayDataType, MLPredictionOptions,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSInteger, NSNumber, NSString, NSURL};

use crate::tensor::{Tensor, is_row_major, offsets, storage_len};

/// Hardware a model may run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComputeUnits {
    CpuOnly,
    CpuAndNeuralEngine,
}

/// A compiled CoreML model (`.mlmodelc`).
pub(crate) struct Model {
    model: Retained<MLModel>,
    options: Retained<MLPredictionOptions>,
}

// SAFETY: MLModel and MLPredictionOptions are not bound to the thread that
// created them; Apple documents MLModel predictions as callable from any
// thread. `Model` is not `Sync`, so it is used by one thread at a time.
unsafe impl Send for Model {}

impl Model {
    /// Loads (and, on first use, compiles for the device) the model at `path`.
    pub(crate) fn load(path: &Path, units: ComputeUnits) -> Result<Model, String> {
        let url = NSURL::from_file_path(path)
            .ok_or_else(|| format!("invalid model path {}", path.display()))?;
        let units = match units {
            ComputeUnits::CpuOnly => MLComputeUnits::CPUOnly,
            ComputeUnits::CpuAndNeuralEngine => MLComputeUnits::CPUAndNeuralEngine,
        };
        // SAFETY: a fresh configuration is configured with valid enum values
        // and passed with a valid file URL.
        let model = guarded("loading the model", || unsafe {
            let config = MLModelConfiguration::new();
            config.setComputeUnits(units);
            config.setAllowLowPrecisionAccumulationOnGPU(true);
            MLModel::modelWithContentsOfURL_configuration_error(&url, &config)
        })?
        .map_err(|error| describe(&error))?;
        // SAFETY: default prediction options have no preconditions.
        let options = guarded("creating prediction options", || unsafe {
            MLPredictionOptions::new()
        })?;
        Ok(Model { model, options })
    }

    /// Runs one synchronous prediction.
    pub(crate) fn predict(&self, input: &Features) -> Result<Features, String> {
        // SAFETY: both arguments are valid objects; the call is synchronous.
        let output = guarded("prediction", || unsafe {
            self.model
                .predictionFromFeatures_options_error(&input.provider, &self.options)
        })?
        .map_err(|error| describe(&error))?;
        Ok(Features { provider: output })
    }
}

/// Named model inputs or outputs.
pub(crate) struct Features {
    provider: Retained<ProtocolObject<dyn MLFeatureProvider>>,
}

// SAFETY: feature providers are plain immutable containers with no thread
// affinity; `Features` is not `Sync`.
unsafe impl Send for Features {}

impl Features {
    /// Inputs that reference `arrays` by name. Writing to an array later
    /// changes what the next prediction sees.
    pub(crate) fn new(arrays: &[(&str, &Array)]) -> Result<Features, String> {
        let keys: Vec<Retained<NSString>> = arrays
            .iter()
            .map(|(name, _)| NSString::from_str(name))
            .collect();
        // SAFETY: wrapping a valid multi-array in a feature value.
        let values: Vec<Retained<MLFeatureValue>> = guarded("wrapping inputs", || {
            arrays
                .iter()
                .map(|(_, array)| unsafe {
                    MLFeatureValue::featureValueWithMultiArray(&array.array)
                })
                .collect()
        })?;
        let key_refs: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
        let value_refs: Vec<&AnyObject> =
            values.iter().map(|value| -> &AnyObject { value }).collect();
        let dictionary = NSDictionary::<NSString, AnyObject>::from_slices(&key_refs, &value_refs);
        // SAFETY: every value in the dictionary is an MLFeatureValue, which
        // is what the initializer expects.
        let provider = guarded("creating inputs", || unsafe {
            MLDictionaryFeatureProvider::initWithDictionary_error(
                MLDictionaryFeatureProvider::alloc(),
                &dictionary,
            )
        })?
        .map_err(|error| describe(&error))?;
        Ok(Features {
            provider: ProtocolObject::from_retained(provider),
        })
    }

    /// The multi-array feature called `name`.
    pub(crate) fn array(&self, name: &str) -> Result<Array, String> {
        let key = NSString::from_str(name);
        // SAFETY: querying a feature provider by name has no preconditions.
        let value = guarded("reading an output", || unsafe {
            self.provider.featureValueForName(&key)
        })?
        .ok_or_else(|| format!("missing output {name}"))?;
        // SAFETY: returns nil for non-array features, handled below.
        let array = guarded("reading an output", || unsafe { value.multiArrayValue() })?
            .ok_or_else(|| format!("output {name} is not a multi-array"))?;
        Ok(Array { array })
    }
}

/// An `MLMultiArray` of `f32` or `i32` scalars.
pub(crate) struct Array {
    array: Retained<MLMultiArray>,
}

// SAFETY: a multi-array owns its buffer and has no thread affinity; `Array`
// is not `Sync`, so the buffer is never accessed from two threads at once.
unsafe impl Send for Array {}

impl Array {
    /// A zeroed `f32` array.
    pub(crate) fn zeros_f32(shape: &[usize]) -> Result<Array, String> {
        let array = Array::new(shape, MLMultiArrayDataType::Float32)?;
        array.write_f32(&vec![0.0; shape.iter().product()])?;
        Ok(array)
    }

    /// A zeroed `i32` array.
    pub(crate) fn zeros_i32(shape: &[usize]) -> Result<Array, String> {
        let array = Array::new(shape, MLMultiArrayDataType::Int32)?;
        array.write_i32(&vec![0; shape.iter().product()])?;
        Ok(array)
    }

    fn new(shape: &[usize], data_type: MLMultiArrayDataType) -> Result<Array, String> {
        let dims: Vec<Retained<NSNumber>> = shape
            .iter()
            .map(|&dim| NSNumber::new_isize(dim as isize))
            .collect();
        let shape = NSArray::from_retained_slice(&dims);
        // SAFETY: a valid shape and a supported data type.
        let array = guarded("creating a multi-array", || unsafe {
            MLMultiArray::initWithShape_dataType_error(MLMultiArray::alloc(), &shape, data_type)
        })?
        .map_err(|error| describe(&error))?;
        Ok(Array { array })
    }

    /// Overwrites the `f32` array with `values` in row-major order.
    pub(crate) fn write_f32(&self, values: &[f32]) -> Result<(), String> {
        self.write(MLMultiArrayDataType::Float32, values)
    }

    /// Overwrites the `i32` array with `values` in row-major order.
    pub(crate) fn write_i32(&self, values: &[i32]) -> Result<(), String> {
        self.write(MLMultiArrayDataType::Int32, values)
    }

    fn write<T: Copy>(&self, data_type: MLMultiArrayDataType, values: &[T]) -> Result<(), String> {
        self.check_type(data_type)?;
        let shape = self.shape()?;
        if values.len() != shape.iter().product::<usize>() {
            return Err(format!(
                "expected {} values, got {}",
                shape.iter().product::<usize>(),
                values.len()
            ));
        }
        let outcome: RefCell<Result<(), String>> = RefCell::new(Ok(()));
        let handler = StackBlock::new(
            |bytes: NonNull<c_void>, size: NSInteger, strides: NonNull<NSArray<NSNumber>>| {
                // SAFETY: CoreML passes a valid strides array for the
                // duration of the call.
                let strides = numbers(unsafe { strides.as_ref() });
                let Some(len) = storage_len(&shape, &strides) else {
                    *outcome.borrow_mut() = Err("invalid strides".to_owned());
                    return;
                };
                if len * size_of::<T>() > size as usize {
                    *outcome.borrow_mut() = Err("buffer smaller than its shape".to_owned());
                    return;
                }
                let base = bytes.cast::<T>().as_ptr();
                if is_row_major(&shape, &strides) {
                    // SAFETY: a row-major buffer of `values.len()` scalars
                    // spans exactly `len` elements, checked against the
                    // buffer size above; the buffer is ours inside the
                    // handler and cannot overlap `values`.
                    unsafe { std::ptr::copy_nonoverlapping(values.as_ptr(), base, values.len()) };
                    return;
                }
                for (value, offset) in values.iter().zip(offsets(&shape, &strides)) {
                    // SAFETY: every offset is below `len`, checked against
                    // the buffer size above; the buffer is ours to mutate
                    // inside the handler.
                    unsafe { base.add(offset).write(*value) };
                }
            },
        );
        // SAFETY: the handler runs synchronously and only touches the buffer
        // within the bounds checked above.
        guarded("writing a multi-array", || unsafe {
            self.array.getMutableBytesWithHandler(&handler)
        })?;
        outcome.into_inner()
    }

    /// Copies an `f32` array out, keeping CoreML's strides.
    pub(crate) fn read_f32(&self) -> Result<Tensor, String> {
        self.check_type(MLMultiArrayDataType::Float32)?;
        let shape = self.shape()?;
        // SAFETY: reading the strides property has no preconditions.
        let strides = numbers(&*guarded("reading strides", || unsafe {
            self.array.strides()
        })?);
        let len = storage_len(&shape, &strides).ok_or_else(|| "invalid strides".to_owned())?;
        Ok(Tensor {
            data: self.read(len)?,
            shape,
            strides,
        })
    }

    /// Copies an `i32` array out in row-major order.
    pub(crate) fn read_i32s(&self) -> Result<Vec<i32>, String> {
        self.check_type(MLMultiArrayDataType::Int32)?;
        let shape = self.shape()?;
        // SAFETY: reading the strides property has no preconditions.
        let strides = numbers(&*guarded("reading strides", || unsafe {
            self.array.strides()
        })?);
        let len = storage_len(&shape, &strides).ok_or_else(|| "invalid strides".to_owned())?;
        let data = self.read::<i32>(len)?;
        if is_row_major(&shape, &strides) {
            return Ok(data[..shape.iter().product::<usize>().min(len)].to_vec());
        }
        Ok(offsets(&shape, &strides)
            .map(|offset| data.get(offset).copied().unwrap_or(0))
            .collect())
    }

    /// The first scalar of an `i32` array.
    pub(crate) fn first_i32(&self) -> Result<i32, String> {
        self.check_type(MLMultiArrayDataType::Int32)?;
        Ok(self.read::<i32>(1)?[0])
    }

    /// The first scalar of an `f32` array.
    pub(crate) fn first_f32(&self) -> Result<f32, String> {
        self.check_type(MLMultiArrayDataType::Float32)?;
        Ok(self.read::<f32>(1)?[0])
    }

    /// The first `len` scalars of storage.
    fn read<T: Copy>(&self, len: usize) -> Result<Vec<T>, String> {
        let outcome: RefCell<Result<Vec<T>, String>> = RefCell::new(Ok(Vec::new()));
        let handler = StackBlock::new(|bytes: NonNull<c_void>, size: NSInteger| {
            if len * size_of::<T>() > size as usize {
                *outcome.borrow_mut() = Err("buffer smaller than its shape".to_owned());
                return;
            }
            // SAFETY: the buffer holds at least `len` scalars of type `T`
            // (checked above) and stays valid for the duration of the call.
            let slice = unsafe { std::slice::from_raw_parts(bytes.cast::<T>().as_ptr(), len) };
            *outcome.borrow_mut() = Ok(slice.to_vec());
        });
        // SAFETY: the handler runs synchronously and reads within bounds.
        guarded("reading a multi-array", || unsafe {
            self.array.getBytesWithHandler(&handler)
        })?;
        outcome.into_inner()
    }

    /// The array's dimensions.
    pub(crate) fn shape(&self) -> Result<Vec<usize>, String> {
        // SAFETY: reading the shape property has no preconditions.
        Ok(numbers(&*guarded("reading a shape", || unsafe {
            self.array.shape()
        })?))
    }

    fn check_type(&self, want: MLMultiArrayDataType) -> Result<(), String> {
        // SAFETY: reading the data type property has no preconditions.
        let actual = guarded("reading a data type", || unsafe { self.array.dataType() })?;
        if actual != want {
            return Err(format!("unexpected multi-array type {actual:?}"));
        }
        Ok(())
    }
}

fn numbers(array: &NSArray<NSNumber>) -> Vec<usize> {
    array
        .iter()
        .map(|number| usize::try_from(number.as_isize()).unwrap_or(usize::MAX))
        .collect()
}

/// Runs `f`, turning an Objective-C exception thrown inside it into an
/// error. Rust panics pass through unchanged.
fn guarded<R>(what: &str, f: impl FnOnce() -> R) -> Result<R, String> {
    objc2::exception::catch(AssertUnwindSafe(f)).map_err(|exception| match exception {
        Some(exception) => format!("{what} raised {exception:?}"),
        None => format!("{what} raised an exception"),
    })
}

fn describe(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

#[cfg(test)]
mod tests {
    use objc2::msg_send;
    use objc2_foundation::NSArray;

    use super::*;

    /// An exception thrown by Objective-C code becomes an error instead of
    /// aborting the process.
    #[test]
    fn guarded_catches_objective_c_exceptions() {
        let array = NSArray::from_retained_slice(&[NSNumber::new_i32(1)]);
        let caught = guarded("indexing", || {
            // SAFETY: out-of-range `objectAtIndex:` raises NSRangeException,
            // which `guarded` catches; nothing is left half-done.
            let _: *mut AnyObject = unsafe { msg_send![&*array, objectAtIndex: 5usize] };
        });
        let Err(error) = caught else {
            panic!("no exception");
        };
        assert!(error.starts_with("indexing raised"), "{error}");
        assert!(error.contains("NSRangeException"), "{error}");
        assert_eq!(guarded("nothing", || 7), Ok(7));
    }
}
