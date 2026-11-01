//! Strided tensors copied out of CoreML, and encoder frames built from them.

/// Scalars copied from a multi-array's storage, with its shape and strides
/// in elements.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tensor {
    pub data: Vec<f32>,
    pub shape: Vec<usize>,
    pub strides: Vec<usize>,
}

impl Tensor {
    /// The scalars in row-major order.
    pub(crate) fn to_row_major(&self) -> Vec<f32> {
        if is_row_major(&self.shape, &self.strides) {
            let count = self.shape.iter().product::<usize>().min(self.data.len());
            return self.data[..count].to_vec();
        }
        offsets(&self.shape, &self.strides)
            .map(|offset| self.data.get(offset).copied().unwrap_or(0.0))
            .collect()
    }
}

/// Elements of storage a strided layout spans; `None` when the strides do
/// not fit the shape or overflow.
pub(crate) fn storage_len(shape: &[usize], strides: &[usize]) -> Option<usize> {
    if shape.len() != strides.len() {
        return None;
    }
    if shape.contains(&0) {
        return Some(0);
    }
    shape
        .iter()
        .zip(strides)
        .try_fold(1usize, |len, (&dim, &stride)| {
            (dim - 1).checked_mul(stride)?.checked_add(len)
        })
}

/// Whether the strides are the contiguous row-major layout of the shape
/// (axes of length 1 may have any stride).
pub(crate) fn is_row_major(shape: &[usize], strides: &[usize]) -> bool {
    if shape.len() != strides.len() {
        return false;
    }
    let mut want = 1;
    for (&dim, &stride) in shape.iter().zip(strides).rev() {
        if dim != 1 && stride != want {
            return false;
        }
        want *= dim;
    }
    true
}

/// Storage offsets of every element, in row-major order.
pub(crate) fn offsets<'a>(
    shape: &'a [usize],
    strides: &'a [usize],
) -> impl Iterator<Item = usize> + 'a {
    let count = if shape.is_empty() {
        0
    } else {
        shape.iter().product()
    };
    let mut index = vec![0usize; shape.len()];
    (0..count).map(move |n| {
        if n > 0 {
            for axis in (0..shape.len()).rev() {
                index[axis] += 1;
                if index[axis] < shape[axis] {
                    break;
                }
                index[axis] = 0;
            }
        }
        index.iter().zip(strides).map(|(i, s)| i * s).sum()
    })
}

/// Encoder output as contiguous per-frame vectors.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EncoderFrames {
    hidden: usize,
    data: Vec<f32>,
}

impl EncoderFrames {
    /// Reads `[1, hidden, time]` or `[1, time, hidden]` output, keeping the
    /// first `valid` frames.
    pub(crate) fn new(tensor: &Tensor, hidden: usize, valid: usize) -> Result<Self, String> {
        let [batch, a, b] = tensor.shape[..] else {
            return Err(format!("encoder output has shape {:?}", tensor.shape));
        };
        if batch != 1 {
            return Err(format!("encoder output has batch {batch}"));
        }
        let (time_axis, hidden_axis) = match (a == hidden, b == hidden) {
            (true, _) => (2, 1),
            (false, true) => (1, 2),
            (false, false) => return Err(format!("encoder output has shape {:?}", tensor.shape)),
        };
        let frames = valid.min(tensor.shape[time_axis]);
        let (time_stride, hidden_stride) = (tensor.strides[time_axis], tensor.strides[hidden_axis]);
        let mut data = Vec::with_capacity(frames * hidden);
        for frame in 0..frames {
            for h in 0..hidden {
                let value = tensor.data.get(frame * time_stride + h * hidden_stride);
                data.push(
                    value
                        .copied()
                        .ok_or("encoder output shorter than its shape")?,
                );
            }
        }
        Ok(EncoderFrames { hidden, data })
    }

    /// Number of frames.
    pub(crate) fn len(&self) -> usize {
        self.data.len() / self.hidden
    }

    /// The hidden vector of `frame`.
    pub(crate) fn frame(&self, frame: usize) -> Option<&[f32]> {
        self.data
            .get(frame * self.hidden..(frame + 1) * self.hidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_len_covers_strided_layouts() {
        struct Case {
            name: &'static str,
            shape: Vec<usize>,
            strides: Vec<usize>,
            want: Option<usize>,
        }
        let cases = [
            Case {
                name: "contiguous",
                shape: vec![2, 1, 3],
                strides: vec![3, 3, 1],
                want: Some(6),
            },
            Case {
                name: "padded rows",
                shape: vec![1, 2, 3],
                strides: vec![8, 4, 1],
                want: Some(7),
            },
            Case {
                name: "empty",
                shape: vec![1, 0],
                strides: vec![1, 1],
                want: Some(0),
            },
            Case {
                name: "rank mismatch",
                shape: vec![2],
                strides: vec![1, 1],
                want: None,
            },
            Case {
                name: "overflow",
                shape: vec![3],
                strides: vec![usize::MAX],
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(storage_len(&tc.shape, &tc.strides), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn row_major_layouts() {
        let cases = [
            (vec![2, 1, 3], vec![3, 3, 1], true),
            (vec![2, 1, 3], vec![3, 99, 1], true),
            (vec![1, 2, 3], vec![8, 4, 1], false),
            (vec![2, 3], vec![1, 2], false),
        ];
        for (shape, strides, want) in cases {
            assert_eq!(
                is_row_major(&shape, &strides),
                want,
                "{shape:?} {strides:?}"
            );
        }
    }

    #[test]
    fn row_major_follows_strides() {
        let tensor = Tensor {
            data: vec![1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0],
            shape: vec![2, 3],
            strides: vec![4, 1],
        };
        assert_eq!(tensor.to_row_major(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn encoder_frames_transpose_hidden_major_output() {
        // [1, hidden=2, time=3]: hidden 0 = 1 2 3, hidden 1 = 4 5 6
        let tensor = Tensor {
            data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            shape: vec![1, 2, 3],
            strides: vec![6, 3, 1],
        };
        let frames = EncoderFrames::new(&tensor, 2, 2).expect("frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames.frame(0), Some(&[1.0, 4.0][..]));
        assert_eq!(frames.frame(1), Some(&[2.0, 5.0][..]));
        assert_eq!(frames.frame(2), None);
    }
}
