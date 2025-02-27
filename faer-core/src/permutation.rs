use assert2::{assert as fancy_assert, debug_assert as fancy_debug_assert};
use reborrow::*;

use crate::{MatMut, MatRef};

#[track_caller]
#[inline]
pub fn swap_cols<T>(mat: MatMut<'_, T>, a: usize, b: usize) {
    let m = mat.nrows();
    let n = mat.ncols();
    fancy_assert!(a < n);
    fancy_assert!(b < n);

    if a == b {
        return;
    }

    let rs = mat.row_stride();
    let cs = mat.col_stride();
    let ptr = mat.as_ptr();

    let ptr_a = ptr.wrapping_offset(cs * a as isize);
    let ptr_b = ptr.wrapping_offset(cs * b as isize);

    if rs == 1 {
        unsafe {
            core::ptr::swap_nonoverlapping(ptr_a, ptr_b, m);
        }
    } else {
        for i in 0..m {
            let offset = rs * i as isize;
            unsafe {
                core::ptr::swap_nonoverlapping(
                    ptr_a.wrapping_offset(offset),
                    ptr_b.wrapping_offset(offset),
                    1,
                );
            }
        }
    }
}

#[track_caller]
#[inline]
pub fn swap_rows<T>(mat: MatMut<'_, T>, a: usize, b: usize) {
    swap_cols(mat.transpose(), a, b)
}

#[derive(Clone, Copy, Debug)]
pub struct PermutationIndicesRef<'a> {
    forward: &'a [usize],
    inverse: &'a [usize],
}

impl<'a> PermutationIndicesRef<'a> {
    /// Returns the permutation as an array.
    #[inline]
    pub fn into_arrays(self) -> (&'a [usize], &'a [usize]) {
        (self.forward, self.inverse)
    }

    #[inline]
    pub fn len(&self) -> usize {
        fancy_debug_assert!(self.inverse.len() == self.forward.len());
        self.forward.len()
    }

    /// Returns the inverse permutation.
    #[inline]
    pub fn inverse(self) -> Self {
        Self {
            forward: self.inverse,
            inverse: self.forward,
        }
    }

    /// Creates a new permutation reference, without checking the validity of the inputs.
    ///
    /// # Safety
    ///
    /// `forward` and `inverse` must have the same length, be valid permutations, and be inverse
    /// permutations of each other.
    #[inline]
    pub unsafe fn new_unchecked(forward: &'a [usize], inverse: &'a [usize]) -> Self {
        Self { forward, inverse }
    }
}

impl<'a> PermutationIndicesMut<'a> {
    /// Returns the permutation as an array.
    #[inline]
    pub unsafe fn into_arrays(self) -> (&'a mut [usize], &'a mut [usize]) {
        (self.forward, self.inverse)
    }

    #[inline]
    pub fn len(&self) -> usize {
        fancy_debug_assert!(self.inverse.len() == self.forward.len());
        self.forward.len()
    }

    /// Returns the inverse permutation.
    #[inline]
    pub fn inverse(self) -> Self {
        Self {
            forward: self.inverse,
            inverse: self.forward,
        }
    }

    /// Creates a new permutation mutable reference, without checking the validity of the inputs.
    ///
    /// # Safety
    ///
    /// `forward` and `inverse` must have the same length, be valid permutations, and be inverse
    /// permutations of each other.
    #[inline]
    pub unsafe fn new_unchecked(forward: &'a mut [usize], inverse: &'a mut [usize]) -> Self {
        Self { forward, inverse }
    }
}

#[derive(Debug)]
pub struct PermutationIndicesMut<'a> {
    forward: &'a mut [usize],
    inverse: &'a mut [usize],
}

impl<'short, 'a> Reborrow<'short> for PermutationIndicesRef<'a> {
    type Target = PermutationIndicesRef<'short>;

    #[inline]
    fn rb(&'short self) -> Self::Target {
        *self
    }
}

impl<'short, 'a> ReborrowMut<'short> for PermutationIndicesRef<'a> {
    type Target = PermutationIndicesRef<'short>;

    #[inline]
    fn rb_mut(&'short mut self) -> Self::Target {
        *self
    }
}

impl<'short, 'a> Reborrow<'short> for PermutationIndicesMut<'a> {
    type Target = PermutationIndicesRef<'short>;

    #[inline]
    fn rb(&'short self) -> Self::Target {
        PermutationIndicesRef {
            forward: &*self.forward,
            inverse: &*self.inverse,
        }
    }
}

impl<'short, 'a> ReborrowMut<'short> for PermutationIndicesMut<'a> {
    type Target = PermutationIndicesMut<'short>;

    #[inline]
    fn rb_mut(&'short mut self) -> Self::Target {
        PermutationIndicesMut {
            forward: &mut *self.forward,
            inverse: &mut *self.inverse,
        }
    }
}

/// Computes a symmetric permutation of the source matrix using the given permutation, and stores
/// the result in the destination matrix.
///
/// Both the source and the destination are interpreted as symmetric matrices, and only their lower
/// triangular part is accessed.
#[track_caller]
pub fn permute_rows_and_cols_symmetric_lower<T: Copy>(
    dst: MatMut<'_, T>,
    src: MatRef<'_, T>,
    perm_indices: PermutationIndicesRef<'_>,
) {
    let mut dst = dst;
    let n = src.nrows();
    fancy_assert!(src.nrows() == src.ncols(), "source matrix must be square",);
    fancy_assert!(
        dst.nrows() == dst.ncols(),
        "destination matrix must be square",
    );
    fancy_assert!(
        src.nrows() == dst.nrows(),
        "source and destination matrices must have the same shape",
    );
    fancy_assert!(
        perm_indices.into_arrays().0.len() == n,
        "permutation must have the same length as the dimension of the matrices"
    );

    let perm = perm_indices.into_arrays().0;
    let src_tril = |i, j| unsafe {
        if i > j {
            src.get_unchecked(i, j)
        } else {
            src.get_unchecked(j, i)
        }
    };
    for j in 0..n {
        for i in j..n {
            unsafe {
                *dst.rb_mut().ptr_in_bounds_at_unchecked(i, j) =
                    *src_tril(*perm.get_unchecked(i), *perm.get_unchecked(j));
            }
        }
    }
}

#[inline]
unsafe fn permute_rows_unchecked<T: Copy>(
    dst: MatMut<'_, T>,
    src: MatRef<'_, T>,
    perm_indices: PermutationIndicesRef<'_>,
) {
    let mut dst = dst;
    let m = src.nrows();
    let n = src.ncols();
    fancy_debug_assert!(
        (src.nrows(), src.ncols()) == (dst.nrows(), dst.ncols()),
        "source and destination matrices must have the same shape",
    );
    fancy_debug_assert!(
        perm_indices.into_arrays().0.len() == m,
        "permutation must have the same length as the number of rows of the matrices"
    );

    let perm = perm_indices.into_arrays().0;

    if dst.row_stride().abs() < dst.col_stride().abs() {
        for j in 0..n {
            for i in 0..m {
                unsafe {
                    *dst.rb_mut().ptr_in_bounds_at_unchecked(i, j) =
                        *src.get_unchecked(*perm.get_unchecked(i), j);
                }
            }
        }
    } else {
        for i in 0..m {
            unsafe {
                let src_i = src.row_unchecked(*perm.get_unchecked(i));
                let dst_i = dst.rb_mut().row_unchecked(i);

                dst_i.cwise().zip_unchecked(src_i).for_each(|dst, src| {
                    *dst = *src;
                });
            }
        }
    }
}

#[inline]
#[track_caller]
pub fn permute_cols<T: Copy>(
    dst: MatMut<'_, T>,
    src: MatRef<'_, T>,
    perm_indices: PermutationIndicesRef<'_>,
) {
    permute_rows(dst.transpose(), src.transpose(), perm_indices);
}

#[inline]
#[track_caller]
pub fn permute_rows<T: Copy>(
    dst: MatMut<'_, T>,
    src: MatRef<'_, T>,
    perm_indices: PermutationIndicesRef<'_>,
) {
    fancy_assert!(
        (src.nrows(), src.ncols()) == (dst.nrows(), dst.ncols()),
        "source and destination matrices must have the same shape",
    );
    fancy_assert!(
        perm_indices.into_arrays().0.len() == src.nrows(),
        "permutation must have the same length as the number of rows of the matrices"
    );

    unsafe { permute_rows_unchecked(dst, src, perm_indices) };
}
