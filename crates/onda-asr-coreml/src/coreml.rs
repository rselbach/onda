//! Thin wrapper over CoreML models and multi-arrays. This is the only module
//! in the crate allowed to use `unsafe`.
//!
//! Every CoreML call runs inside [`guarded`]: an Objective-C exception
//! unwinding into Rust aborts the process, so it becomes an error instead.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
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

    /// Runs one synchronous prediction with pre-bound owned inputs.
    pub(crate) fn predict_bound(&self, input: &BoundInputs) -> Result<Features, String> {
        self.predict_provider(&input.provider)
    }

    /// Runs one synchronous prediction borrowing temporary input arrays for
    /// the duration of the call.
    pub(crate) fn predict_borrowed(
        &self,
        arrays: &[(&str, &ArrayRef<'_>)],
    ) -> Result<Features, String> {
        let pairs: Vec<(&str, &MLMultiArray)> = arrays
            .iter()
            .map(|(name, array)| (*name, &*array.array))
            .collect();
        let provider = build_provider(&pairs)?;
        self.predict_provider(&provider)
    }

    /// Runs one synchronous prediction forwarding outputs from an earlier step.
    pub(crate) fn predict(&self, input: &Features) -> Result<Features, String> {
        self.predict_provider(&input.provider)
    }

    fn predict_provider(
        &self,
        provider: &ProtocolObject<dyn MLFeatureProvider>,
    ) -> Result<Features, String> {
        // SAFETY: both arguments are valid objects; the call is synchronous and
        // any borrowed inputs remain valid for the whole call.
        let output = guarded("prediction", || unsafe {
            self.model
                .predictionFromFeatures_options_error(provider, &self.options)
        })?
        .map_err(|error| describe(&error))?;
        Ok(Features { provider: output })
    }
}

/// Owned `MLMultiArray` buffers bound into a CoreML feature provider.
///
/// The provider holds references to the same backing arrays, so mutations only
/// go through `&mut BoundInputs`, preventing concurrent access with prediction.
pub(crate) struct BoundInputs {
    arrays: Vec<Array>,
    provider: Retained<ProtocolObject<dyn MLFeatureProvider>>,
}

// SAFETY: owned arrays and an immutable dictionary feature provider over them
// have no thread affinity; `BoundInputs` is not `Sync`.
unsafe impl Send for BoundInputs {}

impl BoundInputs {
    /// Wraps `named` arrays into a reusable feature provider.
    pub(crate) fn new(named: Vec<(&str, Array)>) -> Result<BoundInputs, String> {
        let pairs: Vec<(&str, &MLMultiArray)> = named
            .iter()
            .map(|(name, array)| (*name, &*array.array))
            .collect();
        let provider = build_provider(&pairs)?;
        let arrays = named.into_iter().map(|(_, array)| array).collect();
        Ok(BoundInputs { arrays, provider })
    }

    /// Overwrites the `f32` input at `index` in row-major order.
    pub(crate) fn write_f32(&mut self, index: usize, values: &[f32]) -> Result<(), String> {
        self.arrays
            .get_mut(index)
            .ok_or_else(|| format!("input index {index} out of range"))?
            .write_f32(values)
    }

    /// Overwrites the `i32` input at `index` in row-major order.
    pub(crate) fn write_i32(&mut self, index: usize, values: &[i32]) -> Result<(), String> {
        self.arrays
            .get_mut(index)
            .ok_or_else(|| format!("input index {index} out of range"))?
            .write_i32(values)
    }
}

/// Read-only named model outputs.
pub(crate) struct Features {
    provider: Retained<ProtocolObject<dyn MLFeatureProvider>>,
}

// SAFETY: feature providers are plain immutable containers with no thread
// affinity; `Features` is not `Sync`.
unsafe impl Send for Features {}

fn build_provider(
    arrays: &[(&str, &MLMultiArray)],
) -> Result<Retained<ProtocolObject<dyn MLFeatureProvider>>, String> {
    let keys: Vec<Retained<NSString>> = arrays
        .iter()
        .map(|(name, _)| NSString::from_str(name))
        .collect();
    // SAFETY: wrapping a valid multi-array in a feature value.
    let values: Vec<Retained<MLFeatureValue>> = guarded("wrapping inputs", || {
        arrays
            .iter()
            .map(|(_, array)| unsafe { MLFeatureValue::featureValueWithMultiArray(array) })
            .collect()
    })?;
    let key_refs: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
    let value_refs: Vec<&AnyObject> = values.iter().map(|value| -> &AnyObject { value }).collect();
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
    Ok(ProtocolObject::from_retained(provider))
}

impl Features {
    /// Read-only handle to the multi-array feature called `name`.
    pub(crate) fn array(&self, name: &str) -> Result<ArrayRef<'_>, String> {
        let key = NSString::from_str(name);
        // SAFETY: querying a feature provider by name has no preconditions.
        let value = guarded("reading an output", || unsafe {
            self.provider.featureValueForName(&key)
        })?
        .ok_or_else(|| format!("missing output {name}"))?;
        // SAFETY: returns nil for non-array features, handled below.
        let array = guarded("reading an output", || unsafe { value.multiArrayValue() })?
            .ok_or_else(|| format!("output {name} is not a multi-array"))?;
        Ok(ArrayRef {
            array,
            _borrow: PhantomData,
        })
    }
}

/// A borrowed read-only `MLMultiArray` view. Not `Send` or `Sync`, so aliased
/// output buffers cannot escape across threads or be mutated.
pub(crate) struct ArrayRef<'a> {
    array: Retained<MLMultiArray>,
    _borrow: PhantomData<&'a MLMultiArray>,
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
        let count = checked_element_count(shape)?;
        let mut array = Array::new(shape, MLMultiArrayDataType::Float32)?;
        array.write_f32(&vec![0.0; count])?;
        Ok(array)
    }

    /// A zeroed `i32` array.
    pub(crate) fn zeros_i32(shape: &[usize]) -> Result<Array, String> {
        let count = checked_element_count(shape)?;
        let mut array = Array::new(shape, MLMultiArrayDataType::Int32)?;
        array.write_i32(&vec![0; count])?;
        Ok(array)
    }

    fn new(shape: &[usize], data_type: MLMultiArrayDataType) -> Result<Array, String> {
        let _ = checked_element_count(shape)?;
        let dims: Vec<Retained<NSNumber>> = shape
            .iter()
            .map(|&dim| {
                isize::try_from(dim)
                    .map(NSNumber::new_isize)
                    .map_err(|_| format!("dimension {dim} overflows isize"))
            })
            .collect::<Result<_, _>>()?;
        let shape = NSArray::from_retained_slice(&dims);
        // SAFETY: a valid shape and a supported data type.
        let array = guarded("creating a multi-array", || unsafe {
            MLMultiArray::initWithShape_dataType_error(MLMultiArray::alloc(), &shape, data_type)
        })?
        .map_err(|error| describe(&error))?;
        Ok(Array { array })
    }

    /// Overwrites the `f32` array with `values` in row-major order.
    pub(crate) fn write_f32(&mut self, values: &[f32]) -> Result<(), String> {
        self.write(MLMultiArrayDataType::Float32, values)
    }

    /// Overwrites the `i32` array with `values` in row-major order.
    pub(crate) fn write_i32(&mut self, values: &[i32]) -> Result<(), String> {
        self.write(MLMultiArrayDataType::Int32, values)
    }

    /// Borrows this owned array as a read-only view.
    pub(crate) fn as_ref(&self) -> ArrayRef<'_> {
        ArrayRef {
            array: self.array.clone(),
            _borrow: PhantomData,
        }
    }

    fn write<T: Scalar>(
        &mut self,
        data_type: MLMultiArrayDataType,
        values: &[T],
    ) -> Result<(), String> {
        self.as_ref().check_type(data_type)?;
        let shape = self.as_ref().shape()?;
        let expected = checked_element_count(&shape)?;
        if values.len() != expected {
            return Err(format!("expected {expected} values, got {}", values.len()));
        }
        let outcome: RefCell<Result<(), String>> =
            RefCell::new(Err("multi-array write handler did not run".to_owned()));
        let handler = StackBlock::new(
            |bytes: NonNull<c_void>, size: NSInteger, strides: NonNull<NSArray<NSNumber>>| {
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    // SAFETY: CoreML passes a valid strides array for the
                    // duration of the call.
                    let strides = numbers(unsafe { strides.as_ref() })?;
                    let len = storage_len(&shape, &strides)
                        .ok_or_else(|| "invalid strides".to_owned())?;
                    let byte_size = usize::try_from(size)
                        .map_err(|_| "negative multi-array buffer size".to_owned())?;
                    let needed = len
                        .checked_mul(size_of::<T>())
                        .ok_or_else(|| "multi-array byte size overflow".to_owned())?;
                    if needed > byte_size {
                        return Err("buffer smaller than its shape".to_owned());
                    }
                    let base = bytes.as_ptr().cast::<u8>();
                    if is_row_major(&shape, &strides) {
                        let copy_bytes = values
                            .len()
                            .checked_mul(size_of::<T>())
                            .ok_or_else(|| "multi-array byte size overflow".to_owned())?;
                        // SAFETY: a row-major buffer of `values.len()` scalars spans
                        // `copy_bytes <= needed <= byte_size`, the buffer is exclusively
                        // owned inside the handler, and `values` cannot overlap it.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                values.as_ptr().cast::<u8>(),
                                base,
                                copy_bytes,
                            );
                        }
                        return Ok(());
                    }
                    for (value, offset) in values.iter().zip(offsets(&shape, &strides)) {
                        let byte_offset = offset
                            .checked_mul(size_of::<T>())
                            .ok_or_else(|| "multi-array offset overflow".to_owned())?;
                        // SAFETY: every offset is below `len`, checked against
                        // `byte_size` above, and `write_unaligned` handles arbitrary
                        // buffer alignment.
                        unsafe { base.add(byte_offset).cast::<T>().write_unaligned(*value) };
                    }
                    Ok(())
                }))
                .unwrap_or_else(|_| Err("multi-array write handler panicked".to_owned()));
                *outcome.borrow_mut() = result;
            },
        );
        // SAFETY: the handler runs synchronously and only touches the buffer
        // within the bounds checked above.
        guarded("writing a multi-array", || unsafe {
            self.array.getMutableBytesWithHandler(&handler)
        })?;
        outcome.into_inner()
    }
}

impl ArrayRef<'_> {
    /// Copies an `f32` array out, keeping CoreML's strides.
    pub(crate) fn read_f32(&self) -> Result<Tensor, String> {
        self.check_type(MLMultiArrayDataType::Float32)?;
        let shape = self.shape()?;
        let _ = checked_element_count(&shape)?;
        // SAFETY: reading the strides property has no preconditions.
        let strides = numbers(&*guarded("reading strides", || unsafe {
            self.array.strides()
        })?)?;
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
        })?)?;
        let len = storage_len(&shape, &strides).ok_or_else(|| "invalid strides".to_owned())?;
        let data = self.read::<i32>(len)?;
        if is_row_major(&shape, &strides) {
            let count = checked_element_count(&shape)?.min(len);
            return Ok(data[..count].to_vec());
        }
        Ok(offsets(&shape, &strides)
            .map(|offset| data.get(offset).copied().unwrap_or(0))
            .collect())
    }

    /// The first scalar of an `i32` array.
    pub(crate) fn first_i32(&self) -> Result<i32, String> {
        self.check_type(MLMultiArrayDataType::Int32)?;
        let shape = self.shape()?;
        if checked_element_count(&shape)? == 0 {
            return Err("empty multi-array".to_owned());
        }
        self.read::<i32>(1)?
            .into_iter()
            .next()
            .ok_or_else(|| "empty multi-array".to_owned())
    }

    /// The first scalar of an `f32` array.
    pub(crate) fn first_f32(&self) -> Result<f32, String> {
        self.check_type(MLMultiArrayDataType::Float32)?;
        let shape = self.shape()?;
        if checked_element_count(&shape)? == 0 {
            return Err("empty multi-array".to_owned());
        }
        self.read::<f32>(1)?
            .into_iter()
            .next()
            .ok_or_else(|| "empty multi-array".to_owned())
    }

    /// The first `len` scalars of storage.
    fn read<T: Scalar>(&self, len: usize) -> Result<Vec<T>, String> {
        let outcome: RefCell<Result<Vec<T>, String>> =
            RefCell::new(Err("multi-array read handler did not run".to_owned()));
        let handler = StackBlock::new(|bytes: NonNull<c_void>, size: NSInteger| {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let byte_size = usize::try_from(size)
                    .map_err(|_| "negative multi-array buffer size".to_owned())?;
                let needed = len
                    .checked_mul(size_of::<T>())
                    .ok_or_else(|| "multi-array byte size overflow".to_owned())?;
                if needed > byte_size {
                    return Err("buffer smaller than its shape".to_owned());
                }
                let mut out = vec![T::default(); len];
                if needed > 0 {
                    // SAFETY: `bytes` stays valid for `byte_size >= needed` bytes during the call,
                    // `out` has `len` initialized `T` elements, and byte-wise copy avoids
                    // alignment assumptions on `bytes`.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<u8>(),
                            out.as_mut_ptr().cast::<u8>(),
                            needed,
                        );
                    }
                }
                Ok(out)
            }))
            .unwrap_or_else(|_| Err("multi-array read handler panicked".to_owned()));
            *outcome.borrow_mut() = result;
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
        numbers(&*guarded("reading a shape", || unsafe {
            self.array.shape()
        })?)
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

/// Primitive scalar types whose bit patterns have no padding bytes and can be
/// safely copied byte-for-byte.
///
/// # Safety
///
/// Implementors must be initialized plain-old-data numeric primitives with no
/// uninitialized padding bytes or invalid bit patterns.
unsafe trait Scalar: Copy + Default {}
unsafe impl Scalar for f32 {}
unsafe impl Scalar for i32 {}

fn checked_element_count(shape: &[usize]) -> Result<usize, String> {
    shape
        .iter()
        .try_fold(1usize, |acc, &dim| acc.checked_mul(dim))
        .ok_or_else(|| "multi-array element count overflows usize".to_owned())
}

fn numbers(array: &NSArray<NSNumber>) -> Result<Vec<usize>, String> {
    array
        .iter()
        .map(|number| {
            usize::try_from(number.as_isize())
                .map_err(|_| format!("negative tensor dimension {}", number.as_isize()))
        })
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

    #[test]
    fn shape_overflow_returns_error() {
        let err = Array::zeros_f32(&[usize::MAX, 2])
            .err()
            .expect("overflowing shape must fail");
        assert!(err.contains("overflow"), "{err}");
    }
}
