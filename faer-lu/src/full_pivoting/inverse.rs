use assert2::{assert as fancy_assert, debug_assert as fancy_debug_assert};
use dyn_stack::{DynStack, SizeOverflow, StackReq};
use faer_core::{
    join_raw, mul::triangular, permutation::PermutationIndicesRef, temp_mat_req, temp_mat_uninit,
    ComplexField, Conj, MatMut, MatRef, Parallelism,
};
use reborrow::*;
use triangular::BlockStructure;

fn invert_impl<T: ComplexField>(
    mut dst: MatMut<'_, T>,
    lu_factors: Option<MatRef<'_, T>>,
    row_perm: PermutationIndicesRef<'_>,
    col_perm: PermutationIndicesRef<'_>,
    parallelism: Parallelism,
    stack: DynStack<'_>,
) {
    let lu_factors = match lu_factors {
        Some(lu_factors) => lu_factors,
        None => dst.rb(),
    };

    let m = lu_factors.nrows();
    let n = lu_factors.ncols();
    fancy_debug_assert!(m == n);
    fancy_debug_assert!(dst.nrows() == n);
    fancy_debug_assert!(dst.ncols() == n);

    temp_mat_uninit! {
        let (mut inv_lu, stack) = unsafe { temp_mat_uninit::<T>(m, n, stack) };
        let (mut inv, _) = unsafe { temp_mat_uninit::<T>(m, n, stack) };
    }

    let rs = inv_lu.row_stride();
    let cs = inv_lu.col_stride();
    let ptr = inv_lu.rb_mut().as_ptr();

    // SAFETY: even though the matrices alias, only the strictly lower triangular part of l_inv is
    // accessed and only the upper triangular part of u_inv is accessed.
    let l_inv = unsafe { MatMut::from_raw_parts(ptr, n, n, rs, cs) };
    let u_inv = unsafe { MatMut::from_raw_parts(ptr, n, n, rs, cs) };

    join_raw(
        |parallelism| {
            faer_core::inverse::invert_unit_lower_triangular_to(
                l_inv,
                lu_factors,
                Conj::No,
                parallelism,
            )
        },
        |parallelism| {
            faer_core::inverse::invert_upper_triangular_to(u_inv, lu_factors, Conj::No, parallelism)
        },
        parallelism,
    );

    let l_inv = inv_lu.rb();
    let u_inv = inv_lu.rb();

    triangular::matmul(
        inv.rb_mut(),
        BlockStructure::Rectangular,
        Conj::No,
        u_inv,
        BlockStructure::TriangularUpper,
        Conj::No,
        l_inv,
        BlockStructure::UnitTriangularLower,
        Conj::No,
        None,
        T::one(),
        parallelism,
    );

    let col_p = row_perm.into_arrays().1;
    let row_p = col_perm.into_arrays().1;
    fancy_assert!(row_p.len() == n);
    fancy_assert!(col_p.len() == n);
    unsafe {
        if dst.row_stride().abs() <= dst.col_stride().abs() {
            for j in 0..n {
                let jj = *col_p.get_unchecked(j);
                for i in 0..m {
                    let ii = *row_p.get_unchecked(i);
                    *dst.rb_mut().ptr_in_bounds_at_unchecked(i, j) =
                        *inv.rb().get_unchecked(ii, jj);
                }
            }
        } else {
            for i in 0..m {
                let ii = *row_p.get_unchecked(i);
                for j in 0..n {
                    let jj = *col_p.get_unchecked(j);
                    *dst.rb_mut().ptr_in_bounds_at_unchecked(i, j) =
                        *inv.rb().get_unchecked(ii, jj);
                }
            }
        }
    }
}

/// Computes the inverse of a matrix, given its full pivoting LU decomposition,
/// and stores the result in `dst`.
///
/// # Panics
///
/// - Panics if the LU factors are not a square matrix.
/// - Panics if the row permutation doesn't have the same dimension as the matrix.
/// - Panics if the column permutation doesn't have the same dimension as the matrix.
/// - Panics if the destination shape doesn't match the shape of the matrix.
/// - Panics if the provided memory in `stack` is insufficient.
#[track_caller]
pub fn invert_to<T: ComplexField>(
    dst: MatMut<'_, T>,
    lu_factors: MatRef<'_, T>,
    row_perm: PermutationIndicesRef<'_>,
    col_perm: PermutationIndicesRef<'_>,
    parallelism: Parallelism,
    stack: DynStack<'_>,
) {
    invert_impl(
        dst,
        Some(lu_factors),
        row_perm,
        col_perm,
        parallelism,
        stack,
    )
}

/// Computes the inverse of a matrix, given its full pivoting LU decomposition,
/// and stores the result in `lu_factors`.
///
/// # Panics
///
/// - Panics if the LU factors are not a square matrix.
/// - Panics if the row permutation doesn't have the same dimension as the matrix.
/// - Panics if the column permutation doesn't have the same dimension as the matrix.
/// - Panics if the destination shape doesn't match the shape of the matrix.
/// - Panics if the provided memory in `stack` is insufficient.
#[track_caller]
pub fn invert_in_place<T: ComplexField>(
    lu_factors: MatMut<'_, T>,
    row_perm: PermutationIndicesRef<'_>,
    col_perm: PermutationIndicesRef<'_>,
    parallelism: Parallelism,
    stack: DynStack<'_>,
) {
    invert_impl(lu_factors, None, row_perm, col_perm, parallelism, stack)
}

/// Computes the size and alignment of required workspace for computing the inverse of a
/// matrix, given its full pivoting LU decomposition.
pub fn invert_req<T: 'static>(
    nrows: usize,
    ncols: usize,
    parallelism: Parallelism,
) -> Result<StackReq, SizeOverflow> {
    let _ = parallelism;
    temp_mat_req::<T>(nrows, ncols)?.try_and(temp_mat_req::<T>(nrows, ncols)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::full_pivoting::compute::{lu_in_place, lu_in_place_req};
    use assert_approx_eq::assert_approx_eq;
    use faer_core::{mul::matmul, Mat, Parallelism};
    use rand::random;

    macro_rules! make_stack {
        ($req: expr) => {
            ::dyn_stack::DynStack::new(&mut ::dyn_stack::GlobalMemBuffer::new($req))
        };
    }

    #[test]
    fn test_inverse() {
        (0..32).chain((1..16).map(|i| i * 32)).for_each(|n| {
            let mat = Mat::with_dims(|_, _| random::<f64>(), n, n);
            let mut lu = mat.clone();
            let mut row_perm = vec![0; n];
            let mut row_perm_inv = vec![0; n];
            let mut col_perm = vec![0; n];
            let mut col_perm_inv = vec![0; n];
            let (_, row_perm, col_perm) = lu_in_place(
                lu.as_mut(),
                &mut row_perm,
                &mut row_perm_inv,
                &mut col_perm,
                &mut col_perm_inv,
                Parallelism::Rayon(0),
                make_stack!(lu_in_place_req::<f64>(
                    n,
                    n,
                    Parallelism::Rayon(0),
                    Default::default()
                )
                .unwrap()),
                Default::default(),
            );
            let mut inv = Mat::zeros(n, n);
            invert_to(
                inv.as_mut(),
                lu.as_ref(),
                row_perm.rb(),
                col_perm.rb(),
                Parallelism::Rayon(0),
                make_stack!(invert_req::<f64>(n, n, Parallelism::Rayon(0)).unwrap()),
            );

            let mut prod = Mat::zeros(n, n);
            matmul(
                prod.as_mut(),
                Conj::No,
                mat.as_ref(),
                Conj::No,
                inv.as_ref(),
                Conj::No,
                None,
                1.0,
                Parallelism::Rayon(0),
            );

            for j in 0..n {
                for i in 0..n {
                    let target = if i == j { 1.0 } else { 0.0 };
                    assert_approx_eq!(prod[(i, j)], target);
                }
            }
        });
    }
}
