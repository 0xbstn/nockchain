use nockvm::interpreter::Context;
use nockvm::jets::util::slot;
use nockvm::jets::{JetErr, Result};
use nockvm::noun::{Atom, IndirectAtom, Noun, D, T};
use rayon::prelude::*;

use crate::form::math::base::*;
use crate::form::poly::*;
use crate::hand::handle::*;
use crate::hand::structs::HoonList;
use crate::jets::utils::jet_err;

/// FFT jet - Fast Fourier Transform for polynomial evaluation
/// This is one of the most critical operations for STARK proof generation
pub fn fft_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let poly = slot(sam, 2)?;
    let omega = slot(sam, 3)?;
    
    let Ok(poly_slice) = BPolySlice::try_from(poly) else {
        return jet_err();
    };
    
    let Ok(omega_atom) = omega.as_atom() else {
        return jet_err();
    };
    let omega_belt: Belt = omega_atom.as_u64()?.into();
    
    let n = poly_slice.len();
    if n == 0 || (n & (n - 1)) != 0 {
        return jet_err(); // n must be power of 2
    }
    
    // Allocate result
    let (res, res_slice): (IndirectAtom, &mut [Belt]) = 
        new_handle_mut_slice(&mut context.stack, Some(n));
    
    // Perform FFT
    fft_internal(poly_slice.0, res_slice, omega_belt);
    
    let res_cell = finalize_poly(&mut context.stack, Some(n), res);
    Ok(res_cell)
}

/// iFFT jet - Inverse Fast Fourier Transform
pub fn ifft_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let evals = slot(sam, 2)?;
    let omega = slot(sam, 3)?;
    
    let Ok(evals_slice) = BPolySlice::try_from(evals) else {
        return jet_err();
    };
    
    let Ok(omega_atom) = omega.as_atom() else {
        return jet_err();
    };
    let omega_belt: Belt = omega_atom.as_u64()?.into();
    
    let n = evals_slice.len();
    if n == 0 || (n & (n - 1)) != 0 {
        return jet_err(); // n must be power of 2
    }
    
    // Allocate result
    let (res, res_slice): (IndirectAtom, &mut [Belt]) = 
        new_handle_mut_slice(&mut context.stack, Some(n));
    
    // Perform iFFT
    ifft_internal(evals_slice.0, res_slice, omega_belt);
    
    let res_cell = finalize_poly(&mut context.stack, Some(n), res);
    Ok(res_cell)
}

/// Internal FFT implementation using Cooley-Tukey algorithm
fn fft_internal(input: &[Belt], output: &mut [Belt], omega: Belt) {
    let n = input.len();
    
    if n == 1 {
        output[0] = input[0];
        return;
    }
    
    // Parallel FFT for large polynomials
    if n >= 1024 {
        fft_parallel(input, output, omega);
    } else {
        fft_recursive(input, output, omega);
    }
}

/// Recursive FFT implementation
fn fft_recursive(input: &[Belt], output: &mut [Belt], omega: Belt) {
    let n = input.len();
    if n == 1 {
        output[0] = input[0];
        return;
    }
    
    let half_n = n / 2;
    let omega_sq = omega * omega;
    
    // Split into even and odd
    let mut even = vec![Belt::ZERO; half_n];
    let mut odd = vec![Belt::ZERO; half_n];
    
    for i in 0..half_n {
        even[i] = input[2 * i];
        odd[i] = input[2 * i + 1];
    }
    
    // Recursive calls
    let mut even_out = vec![Belt::ZERO; half_n];
    let mut odd_out = vec![Belt::ZERO; half_n];
    
    fft_recursive(&even, &mut even_out, omega_sq);
    fft_recursive(&odd, &mut odd_out, omega_sq);
    
    // Combine results
    let mut omega_power = Belt::ONE;
    for i in 0..half_n {
        let t = omega_power * odd_out[i];
        output[i] = even_out[i] + t;
        output[i + half_n] = even_out[i] - t;
        omega_power = omega_power * omega;
    }
}

/// Parallel FFT for large polynomials
fn fft_parallel(input: &[Belt], output: &mut [Belt], omega: Belt) {
    let n = input.len();
    let log_n = n.trailing_zeros() as usize;
    
    // Bit reversal using parallel iterator
    let mut working = vec![Belt::ZERO; n];
    working.par_iter_mut().enumerate().for_each(|(i, val)| {
        let rev_i = reverse_bits(i, log_n);
        *val = input[rev_i];
    });
    
    // Cooley-Tukey iterative FFT
    let mut size = 2;
    while size <= n {
        let half_size = size / 2;
        let omega_step = omega.pow((n / size) as u64);
        
        (0..n).into_par_iter().step_by(size).for_each(|k| {
            let mut omega_power = Belt::ONE;
            for j in 0..half_size {
                let t = omega_power * working[k + j + half_size];
                let u = working[k + j];
                
                unsafe {
                    let working_ptr = &working as *const Vec<Belt> as *mut Vec<Belt>;
                    (*working_ptr)[k + j] = u + t;
                    (*working_ptr)[k + j + half_size] = u - t;
                }
                
                omega_power = omega_power * omega_step;
            }
        });
        
        size *= 2;
    }
    
    output.copy_from_slice(&working);
}

/// iFFT implementation
fn ifft_internal(input: &[Belt], output: &mut [Belt], omega: Belt) {
    let n = input.len();
    let n_inv = Belt::from(n as u64).inv();
    let omega_inv = omega.inv();
    
    // Perform forward FFT with inverse omega
    fft_internal(input, output, omega_inv);
    
    // Scale by 1/n
    output.par_iter_mut().for_each(|val| {
        *val = *val * n_inv;
    });
}

/// Reverse bits for FFT bit-reversal
fn reverse_bits(n: usize, bits: usize) -> usize {
    let mut result = 0;
    let mut n = n;
    for _ in 0..bits {
        result = (result << 1) | (n & 1);
        n >>= 1;
    }
    result
}

/// Batch FFT jet - perform multiple FFTs in parallel
pub fn batch_fft_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let polys = slot(sam, 2)?;
    let omega = slot(sam, 3)?;
    
    let Ok(omega_atom) = omega.as_atom() else {
        return jet_err();
    };
    let omega_belt: Belt = omega_atom.as_u64()?.into();
    
    // Convert list of polynomials
    let poly_list: Vec<BPolySlice> = HoonList::try_from(polys)?
        .into_iter()
        .filter_map(|p| BPolySlice::try_from(p).ok())
        .collect();
    
    if poly_list.is_empty() {
        return Ok(Noun::from(0u64));
    }
    
    let n = poly_list[0].len();
    
    // Allocate results
    let results: Vec<_> = poly_list
        .par_iter()
        .map(|poly| {
            let mut output = vec![Belt::ZERO; n];
            fft_internal(poly.0, &mut output, omega_belt);
            output
        })
        .collect();
    
    // Convert results back to noun
    let mut res_list = Noun::from(0u64);
    for result in results.iter().rev() {
        let (res_atom, res_slice): (IndirectAtom, &mut [Belt]) = 
            new_handle_mut_slice(&mut context.stack, Some(n));
        res_slice.copy_from_slice(result);
        let res_poly = finalize_poly(&mut context.stack, Some(n), res_atom);
        res_list = T(&mut context.stack, &[res_poly, res_list]);
    }
    
    Ok(res_list)
}