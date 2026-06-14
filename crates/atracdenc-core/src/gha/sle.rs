//! Small dense linear-equation solver (Gaussian elimination).
//!
//! Direct port of `sle.c` from libgha (Daniil Cherednik, LGPL-2.1-or-later).
//! The matrix `a` is an `n x (n+1)` augmented system stored row-major; on
//! success the solution is written into `x[0..n]`. Returns `-1` if the system
//! is singular (or empty), `0` otherwise. Internals use `f64` exactly as C.

/// Gaussian elimination with partial pivoting.
///
/// `a` is row-major with `n + 1` columns (the augmented matrix). `x` must have
/// length at least `n`.
pub fn sle_solve(a: &mut [f64], n: usize, x: &mut [f64]) -> i32 {
    let col = n + 1;
    let eps: f32 = 0.00001;

    if n == 0 {
        return -1;
    }

    for k in 0..n {
        let mut max = (a[col * k + k]).abs();
        let mut index = k;
        for i in (k + 1)..n {
            let t = (a[col * i + k]).abs();
            if t > max {
                max = t;
                index = i;
            }
        }
        if max < eps as f64 {
            return -1;
        }
        if index != k {
            for i in 0..col {
                a.swap(col * k + i, col * index + i);
            }
        }
        for i in k..n {
            let t = a[col * i + k];
            if t.abs() < eps as f64 {
                continue;
            }
            for j in 0..col {
                a[i * col + j] /= t;
            }
            if i != k {
                for j in 0..col {
                    a[i * col + j] -= a[k * col + j];
                }
            }
        }
    }

    for k in (0..n).rev() {
        x[k] = a[col * k + n];
        for i in 0..k {
            a[col * i + n] -= a[col * i + k] * x[k];
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sle_ok_1() {
        // From libgha test/ut.c: eq_matrix_1 / eq_result_1
        let mut m = [
            2.0, 1.0, -1.0, 8.0, //
            -3.0, -1.0, 2.0, -11.0, //
            -2.0, 1.0, 2.0, -3.0,
        ];
        let mut x = [0.0f64; 3];
        let rv = sle_solve(&mut m, 3, &mut x);
        assert_eq!(rv, 0);
        let expected = [2.0, 3.0, -1.0];
        for (got, exp) in x.iter().zip(expected) {
            assert!((got - exp).abs() < 1e-9, "{got} != {exp}");
        }
    }

    #[test]
    fn sle_not_ok_2() {
        // Singular / rank-deficient system must return -1.
        let mut m = [
            2.0, 1.0, 1.0, //
            4.0, 2.0, 2.0,
        ];
        let mut x = [0.0f64; 2];
        let rv = sle_solve(&mut m, 2, &mut x);
        assert_eq!(rv, -1);
    }
}
