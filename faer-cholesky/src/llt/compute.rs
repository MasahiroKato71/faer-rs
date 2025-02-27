use assert2::{assert as fancy_assert, debug_assert as fancy_debug_assert};
use dyn_stack::{DynStack, SizeOverflow, StackReq};
use faer_core::{
    mul::triangular::BlockStructure, parallelism_degree, solve, ComplexField, Conj, MatMut,
    Parallelism,
};
use reborrow::*;

use super::CholeskyError;

fn cholesky_in_place_left_looking_impl<T: ComplexField>(
    matrix: MatMut<'_, T>,
    block_size: usize,
    parallelism: Parallelism,
) -> Result<(), CholeskyError> {
    let mut matrix = matrix;
    fancy_debug_assert!(
        matrix.ncols() == matrix.nrows(),
        "only square matrices can be decomposed into cholesky factors",
    );

    let n = matrix.nrows();

    match n {
        0 => return Ok(()),
        1 => {
            let elem = &mut matrix[(0, 0)];
            let real = (*elem).into_real_imag().0;
            return if real > T::Real::zero() {
                *elem = T::from_real(real.sqrt());
                Ok(())
            } else {
                Err(CholeskyError)
            };
        }
        _ => (),
    };

    let mut idx = 0;
    loop {
        let block_size = (n - idx).min(block_size);

        let (_, _, bottom_left, bottom_right) = matrix.rb_mut().split_at(idx, idx);
        let (_, l10, _, l20) = bottom_left.into_const().split_at(block_size, 0);
        let (mut a11, _, mut a21, _) = bottom_right.split_at(block_size, block_size);

        //
        //      L00
        // A =  L10  A11
        //      L20  A21  A22
        //
        // the first column block is already computed
        // we now compute A11 and A21
        //
        // L00           L00^H L10^H L20^H
        // L10 L11             L11^H L21^H
        // L20 L21 L22 ×             L22^H
        //
        //
        // L00×L00^H
        // L10×L00^H  L10×L10^H + L11×L11^H
        // L20×L00^H  L20×L10^H + L21×L11^H  L20×L20^H + L21×L21^H + L22×L22^H

        // A11 -= L10 × L10^H
        if l10.ncols() > 0 {
            faer_core::mul::triangular::matmul(
                a11.rb_mut(),
                BlockStructure::TriangularLower,
                Conj::No,
                l10,
                BlockStructure::Rectangular,
                Conj::No,
                l10.transpose(),
                BlockStructure::Rectangular,
                Conj::Yes,
                Some(T::one()),
                -T::one(),
                parallelism,
            );
        }

        cholesky_in_place_left_looking_impl(a11.rb_mut(), block_size / 2, parallelism)?;

        if idx + block_size == n {
            break;
        }

        let ld11 = a11.into_const();
        let l11 = ld11;

        // A21 -= L20 × L10^H
        faer_core::mul::matmul(
            a21.rb_mut(),
            Conj::No,
            l20,
            Conj::No,
            l10.transpose(),
            Conj::Yes,
            Some(T::one()),
            -T::one(),
            parallelism,
        );

        // A21 is now L21×L11^H
        // find L21
        //
        // conj(L11) L21^T = A21^T

        solve::solve_lower_triangular_in_place(
            l11,
            Conj::Yes,
            a21.rb_mut().transpose(),
            Conj::No,
            parallelism,
        );

        idx += block_size;
    }
    Ok(())
}

#[derive(Default, Copy, Clone)]
#[non_exhaustive]
pub struct LltParams {}

/// Computes the size and alignment of required workspace for performing a Cholesky
/// decomposition with partial pivoting.
pub fn cholesky_in_place_req<T: 'static>(
    dim: usize,
    parallelism: Parallelism,
    params: LltParams,
) -> Result<StackReq, SizeOverflow> {
    let _ = dim;
    let _ = parallelism;
    let _ = params;
    Ok(StackReq::default())
}

fn cholesky_in_place_impl<T: ComplexField>(
    matrix: MatMut<'_, T>,
    parallelism: Parallelism,
    stack: DynStack<'_>,
) -> Result<(), CholeskyError> {
    // right looking cholesky

    fancy_debug_assert!(matrix.nrows() == matrix.ncols());
    let mut matrix = matrix;
    let mut stack = stack;

    let n = matrix.nrows();
    if n < 4 {
        cholesky_in_place_left_looking_impl(matrix, 1, parallelism)
    } else {
        let block_size = (n / 2).min(128 * parallelism_degree(parallelism));
        let (mut l00, _, mut a10, mut a11) = matrix.rb_mut().split_at(block_size, block_size);

        cholesky_in_place_impl(l00.rb_mut(), parallelism, stack.rb_mut())?;

        let l00 = l00.into_const();

        solve::solve_lower_triangular_in_place(
            l00,
            Conj::Yes,
            a10.rb_mut().transpose(),
            Conj::No,
            parallelism,
        );

        faer_core::mul::triangular::matmul(
            a11.rb_mut(),
            BlockStructure::TriangularLower,
            Conj::No,
            a10.rb(),
            BlockStructure::Rectangular,
            Conj::No,
            a10.rb().transpose(),
            BlockStructure::Rectangular,
            Conj::Yes,
            Some(T::one()),
            -T::one(),
            parallelism,
        );

        cholesky_in_place_impl(a11, parallelism, stack)
    }
}

/// Computes the Cholesky factor $L$ of a hermitian positive definite input matrix $A$ such that
/// $L$ is lower triangular, and
/// $$LL^* == A.$$
///
/// The result is stored back in the same matrix, or an error is returned if the matrix is not
/// positive definite.
///
/// The input matrix is interpreted as symmetric and only the lower triangular part is read.
///
/// The strictly upper triangular part of the matrix is clobbered and may be filled with garbage
/// values.
///
/// # Panics
///
/// - Panics if the input matrix is not square.
/// - Panics if the provided memory in `stack` is insufficient.
#[track_caller]
#[inline]
pub fn cholesky_in_place<T: ComplexField>(
    matrix: MatMut<'_, T>,
    parallelism: Parallelism,
    stack: DynStack<'_>,
    params: LltParams,
) -> Result<(), CholeskyError> {
    let _ = params;
    fancy_assert!(
        matrix.ncols() == matrix.nrows(),
        "only square matrices can be decomposed into cholesky factors",
    );
    cholesky_in_place_impl(matrix, parallelism, stack)
}
