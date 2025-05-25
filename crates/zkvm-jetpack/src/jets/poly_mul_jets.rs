use nockvm::interpreter::Context;
use nockvm::jets::util::slot;
use nockvm::jets::{JetErr, Result};
use nockvm::noun::{IndirectAtom, Noun, T};
use rayon::prelude::*;

use crate::form::math::base::*;
use crate::form::poly::*;
use crate::hand::handle::*;
use crate::hand::structs::HoonList;
use crate::jets::utils::jet_err;

/// Polynomial multiplication using FFT
/// This is much faster than naive O(n²) multiplication for large polynomials
pub fn bpmul_fft_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let p = slot(sam, 2)?;
    let q = slot(sam, 3)?;
    
    let (Ok(p_poly), Ok(q_poly)) = (BPolySlice::try_from(p), BPolySlice::try_from(q)) else {
        return jet_err();
    };
    
    // Handle zero polynomials
    if p_poly.is_empty() || q_poly.is_empty() {
        return Ok(Noun::from(0u64));
    }
    
    let p_len = p_poly.len();
    let q_len = q_poly.len();
    let result_len = p_len + q_len - 1;
    
    // For small polynomials, use naive multiplication
    if result_len < 64 {
        return bpmul_naive_jet(context, subject);
    }
    
    // Find next power of 2
    let fft_size = result_len.next_power_of_two();
    
    // Pad polynomials to FFT size
    let mut p_padded = vec![Belt::ZERO; fft_size];
    let mut q_padded = vec![Belt::ZERO; fft_size];
    
    p_padded[..p_len].copy_from_slice(p_poly.0);
    q_padded[..q_len].copy_from_slice(q_poly.0);
    
    // Find appropriate root of unity
    let omega = find_root_of_unity(fft_size)?;
    
    // Perform FFT on both polynomials
    let mut p_fft = vec![Belt::ZERO; fft_size];
    let mut q_fft = vec![Belt::ZERO; fft_size];
    
    fft_for_mul(&p_padded, &mut p_fft, omega);
    fft_for_mul(&q_padded, &mut q_fft, omega);
    
    // Pointwise multiplication
    let mut product_fft: Vec<Belt> = p_fft
        .par_iter()
        .zip(q_fft.par_iter())
        .map(|(&a, &b)| a * b)
        .collect();
    
    // Inverse FFT
    let omega_inv = omega.inv();
    let mut result = vec![Belt::ZERO; fft_size];
    fft_for_mul(&product_fft, &mut result, omega_inv);
    
    // Scale by 1/n
    let n_inv = Belt::from(fft_size as u64).inv();
    result.par_iter_mut().for_each(|val| {
        *val = *val * n_inv;
    });
    
    // Allocate and copy result (trim to actual size)
    let (res, res_slice): (IndirectAtom, &mut [Belt]) = 
        new_handle_mut_slice(&mut context.stack, Some(result_len));
    res_slice.copy_from_slice(&result[..result_len]);
    
    let res_cell = finalize_poly(&mut context.stack, Some(result_len), res);
    Ok(res_cell)
}

/// Naive polynomial multiplication for small polynomials
pub fn bpmul_naive_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let p = slot(sam, 2)?;
    let q = slot(sam, 3)?;
    
    let (Ok(p_poly), Ok(q_poly)) = (BPolySlice::try_from(p), BPolySlice::try_from(q)) else {
        return jet_err();
    };
    
    if p_poly.is_empty() || q_poly.is_empty() {
        return Ok(Noun::from(0u64));
    }
    
    let result_len = p_poly.len() + q_poly.len() - 1;
    let (res, res_slice): (IndirectAtom, &mut [Belt]) = 
        new_handle_mut_slice(&mut context.stack, Some(result_len));
    
    // Initialize to zero
    res_slice.fill(Belt::ZERO);
    
    // Naive O(n²) multiplication
    for (i, &p_coeff) in p_poly.0.iter().enumerate() {
        for (j, &q_coeff) in q_poly.0.iter().enumerate() {
            res_slice[i + j] = res_slice[i + j] + (p_coeff * q_coeff);
        }
    }
    
    let res_cell = finalize_poly(&mut context.stack, Some(result_len), res);
    Ok(res_cell)
}

/// Find appropriate root of unity for FFT size
fn find_root_of_unity(n: usize) -> Result<Belt> {
    // For the prime field used in STARKs (2^64 - 2^32 + 1)
    // we need to find an nth root of unity
    
    if n > (1 << 32) {
        return Err(JetErr::Deterministic);
    }
    
    // Generator for multiplicative group
    let g = Belt::from(7u64);
    
    // Order of multiplicative group
    let group_order = (1u64 << 64) - (1u64 << 32);
    let exponent = group_order / (n as u64);
    
    Ok(g.pow(exponent))
}

/// FFT optimized for polynomial multiplication
fn fft_for_mul(input: &[Belt], output: &mut [Belt], omega: Belt) {
    let n = input.len();
    let log_n = n.trailing_zeros() as usize;
    
    // Copy with bit reversal
    for i in 0..n {
        let rev_i = reverse_bits(i, log_n);
        output[rev_i] = input[i];
    }
    
    // Cooley-Tukey FFT
    let mut size = 2;
    while size <= n {
        let half_size = size / 2;
        let omega_step = omega.pow((n / size) as u64);
        
        for k in (0..n).step_by(size) {
            let mut omega_power = Belt::ONE;
            for j in 0..half_size {
                let t = omega_power * output[k + j + half_size];
                let u = output[k + j];
                
                output[k + j] = u + t;
                output[k + j + half_size] = u - t;
                
                omega_power = omega_power * omega_step;
            }
        }
        
        size *= 2;
    }
}

/// Batch polynomial multiplication
pub fn batch_bpmul_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let pairs = slot(sam, 2)?;
    
    // Parse list of polynomial pairs
    let pair_list: Vec<(BPolySlice, BPolySlice)> = HoonList::try_from(pairs)?
        .into_iter()
        .filter_map(|pair| {
            if let Ok(cell) = pair.as_cell() {
                if let (Ok(p), Ok(q)) = (
                    BPolySlice::try_from(cell.head()),
                    BPolySlice::try_from(cell.tail())
                ) {
                    return Some((p, q));
                }
            }
            None
        })
        .collect();
    
    if pair_list.is_empty() {
        return Ok(Noun::from(0u64));
    }
    
    // Parallel multiplication
    let results: Vec<Vec<Belt>> = pair_list
        .par_iter()
        .map(|(p, q)| multiply_polynomials(p.0, q.0))
        .collect();
    
    // Convert results back to noun list
    let mut res_list = Noun::from(0u64);
    for result in results.iter().rev() {
        let (res_atom, res_slice): (IndirectAtom, &mut [Belt]) = 
            new_handle_mut_slice(&mut context.stack, Some(result.len()));
        res_slice.copy_from_slice(result);
        let res_poly = finalize_poly(&mut context.stack, Some(result.len()), res_atom);
        res_list = T(&mut context.stack, &[res_poly, res_list]);
    }
    
    Ok(res_list)
}

/// Core polynomial multiplication logic
fn multiply_polynomials(p: &[Belt], q: &[Belt]) -> Vec<Belt> {
    if p.is_empty() || q.is_empty() {
        return vec![];
    }
    
    let result_len = p.len() + q.len() - 1;
    
    // Use FFT for large polynomials
    if result_len >= 64 {
        multiply_fft(p, q)
    } else {
        multiply_naive(p, q)
    }
}

/// Naive multiplication for small polynomials
fn multiply_naive(p: &[Belt], q: &[Belt]) -> Vec<Belt> {
    let result_len = p.len() + q.len() - 1;
    let mut result = vec![Belt::ZERO; result_len];
    
    for (i, &p_coeff) in p.iter().enumerate() {
        for (j, &q_coeff) in q.iter().enumerate() {
            result[i + j] = result[i + j] + (p_coeff * q_coeff);
        }
    }
    
    result
}

/// FFT multiplication for large polynomials
fn multiply_fft(p: &[Belt], q: &[Belt]) -> Vec<Belt> {
    let result_len = p.len() + q.len() - 1;
    let fft_size = result_len.next_power_of_two();
    
    let mut p_padded = vec![Belt::ZERO; fft_size];
    let mut q_padded = vec![Belt::ZERO; fft_size];
    
    p_padded[..p.len()].copy_from_slice(p);
    q_padded[..q.len()].copy_from_slice(q);
    
    let omega = find_root_of_unity(fft_size).unwrap_or(Belt::from(1u64));
    
    let mut p_fft = vec![Belt::ZERO; fft_size];
    let mut q_fft = vec![Belt::ZERO; fft_size];
    
    fft_for_mul(&p_padded, &mut p_fft, omega);
    fft_for_mul(&q_padded, &mut q_fft, omega);
    
    let mut product_fft: Vec<Belt> = p_fft
        .iter()
        .zip(q_fft.iter())
        .map(|(&a, &b)| a * b)
        .collect();
    
    let omega_inv = omega.inv();
    let mut result_padded = vec![Belt::ZERO; fft_size];
    fft_for_mul(&product_fft, &mut result_padded, omega_inv);
    
    let n_inv = Belt::from(fft_size as u64).inv();
    let mut result = vec![Belt::ZERO; result_len];
    
    for i in 0..result_len {
        result[i] = result_padded[i] * n_inv;
    }
    
    result
}

fn reverse_bits(n: usize, bits: usize) -> usize {
    let mut result = 0;
    let mut n = n;
    for _ in 0..bits {
        result = (result << 1) | (n & 1);
        n >>= 1;
    }
    result
}