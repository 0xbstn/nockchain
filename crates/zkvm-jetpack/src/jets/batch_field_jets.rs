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

/// Batch field addition - add multiple field elements in parallel
pub fn batch_badd_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let list_a = slot(sam, 2)?;
    let list_b = slot(sam, 3)?;
    
    let a_vec: Vec<Belt> = parse_belt_list(list_a)?;
    let b_vec: Vec<Belt> = parse_belt_list(list_b)?;
    
    if a_vec.len() != b_vec.len() {
        return jet_err();
    }
    
    // Parallel addition
    let results: Vec<Belt> = a_vec
        .par_iter()
        .zip(b_vec.par_iter())
        .map(|(&a, &b)| a + b)
        .collect();
    
    belt_vec_to_noun_list(context, &results)
}

/// Batch field multiplication - multiply multiple field elements in parallel
pub fn batch_bmul_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let list_a = slot(sam, 2)?;
    let list_b = slot(sam, 3)?;
    
    let a_vec: Vec<Belt> = parse_belt_list(list_a)?;
    let b_vec: Vec<Belt> = parse_belt_list(list_b)?;
    
    if a_vec.len() != b_vec.len() {
        return jet_err();
    }
    
    // Parallel multiplication
    let results: Vec<Belt> = a_vec
        .par_iter()
        .zip(b_vec.par_iter())
        .map(|(&a, &b)| a * b)
        .collect();
    
    belt_vec_to_noun_list(context, &results)
}

/// Batch field inversion - compute multiplicative inverses in batch
/// Uses Montgomery's trick for efficiency
pub fn batch_binv_jet(context: &mut Context, subject: Noun) -> Result {
    let elements = slot(subject, 6)?;
    let belt_vec: Vec<Belt> = parse_belt_list(elements)?;
    
    if belt_vec.is_empty() {
        return Ok(D(0));
    }
    
    // Montgomery's batch inversion trick
    let results = batch_invert(&belt_vec)?;
    
    belt_vec_to_noun_list(context, &results)
}

/// Batch field exponentiation
pub fn batch_bpow_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let bases = slot(sam, 2)?;
    let exponent = slot(sam, 3)?;
    
    let base_vec: Vec<Belt> = parse_belt_list(bases)?;
    let exp_atom = exponent.as_atom()?;
    let exp = exp_atom.as_u64()?;
    
    // Parallel exponentiation
    let results: Vec<Belt> = base_vec
        .par_iter()
        .map(|&base| base.pow(exp))
        .collect();
    
    belt_vec_to_noun_list(context, &results)
}

/// Batch linear combination: compute sum(a_i * b_i) for vectors a and b
pub fn batch_linear_combination_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let coeffs = slot(sam, 2)?;
    let values = slot(sam, 3)?;
    
    let coeff_vec: Vec<Belt> = parse_belt_list(coeffs)?;
    let value_vec: Vec<Belt> = parse_belt_list(values)?;
    
    if coeff_vec.len() != value_vec.len() {
        return jet_err();
    }
    
    // Parallel computation with reduction
    let result = coeff_vec
        .par_iter()
        .zip(value_vec.par_iter())
        .map(|(&c, &v)| c * v)
        .reduce(|| Belt::ZERO, |a, b| a + b);
    
    Ok(Atom::new(&mut context.stack, result.into()).as_noun())
}

/// Multi-scalar multiplication using Pippenger's algorithm
pub fn batch_msm_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let scalars = slot(sam, 2)?;
    let points = slot(sam, 3)?;
    
    let scalar_vec: Vec<Belt> = parse_belt_list(scalars)?;
    let point_vec: Vec<Belt> = parse_belt_list(points)?;
    
    if scalar_vec.len() != point_vec.len() {
        return jet_err();
    }
    
    // For field elements, MSM is just dot product
    let result = scalar_vec
        .par_iter()
        .zip(point_vec.par_iter())
        .map(|(&s, &p)| s * p)
        .reduce(|| Belt::ZERO, |a, b| a + b);
    
    Ok(Atom::new(&mut context.stack, result.into()).as_noun())
}

/// Batch field subtraction
pub fn batch_bsub_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let list_a = slot(sam, 2)?;
    let list_b = slot(sam, 3)?;
    
    let a_vec: Vec<Belt> = parse_belt_list(list_a)?;
    let b_vec: Vec<Belt> = parse_belt_list(list_b)?;
    
    if a_vec.len() != b_vec.len() {
        return jet_err();
    }
    
    // Parallel subtraction
    let results: Vec<Belt> = a_vec
        .par_iter()
        .zip(b_vec.par_iter())
        .map(|(&a, &b)| a - b)
        .collect();
    
    belt_vec_to_noun_list(context, &results)
}

/// Batch evaluation of polynomial at multiple points
pub fn batch_poly_eval_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let poly = slot(sam, 2)?;
    let points = slot(sam, 3)?;
    
    let poly_coeffs = BPolySlice::try_from(poly)?;
    let eval_points: Vec<Belt> = parse_belt_list(points)?;
    
    // Parallel evaluation using Horner's method
    let results: Vec<Belt> = eval_points
        .par_iter()
        .map(|&x| evaluate_polynomial(poly_coeffs.0, x))
        .collect();
    
    belt_vec_to_noun_list(context, &results)
}

// Helper functions

/// Parse a Hoon list into a vector of Belt elements
fn parse_belt_list(list: Noun) -> Result<Vec<Belt>> {
    HoonList::try_from(list)?
        .into_iter()
        .map(|elem| {
            elem.as_atom()
                .and_then(|a| a.as_u64())
                .map(|u| Belt::from(u))
                .ok_or(JetErr::Deterministic)
        })
        .collect()
}

/// Convert a vector of Belt elements to a Hoon list
fn belt_vec_to_noun_list(context: &mut Context, belts: &[Belt]) -> Result {
    let mut result = D(0);
    
    for &belt in belts.iter().rev() {
        let atom = Atom::new(&mut context.stack, belt.into());
        result = T(&mut context.stack, &[atom.as_noun(), result]);
    }
    
    Ok(result)
}

/// Montgomery's batch inversion trick
fn batch_invert(elements: &[Belt]) -> Result<Vec<Belt>> {
    if elements.is_empty() {
        return Ok(vec![]);
    }
    
    let n = elements.len();
    let mut products = vec![Belt::ONE; n];
    let mut results = vec![Belt::ZERO; n];
    
    // Forward pass: compute cumulative products
    products[0] = elements[0];
    for i in 1..n {
        products[i] = products[i - 1] * elements[i];
    }
    
    // Compute inverse of final product
    let mut inv = products[n - 1].inv();
    
    // Backward pass: compute individual inverses
    for i in (1..n).rev() {
        results[i] = products[i - 1] * inv;
        inv = inv * elements[i];
    }
    results[0] = inv;
    
    Ok(results)
}

/// Evaluate polynomial using Horner's method
fn evaluate_polynomial(coeffs: &[Belt], x: Belt) -> Belt {
    if coeffs.is_empty() {
        return Belt::ZERO;
    }
    
    let mut result = coeffs[coeffs.len() - 1];
    
    for i in (0..coeffs.len() - 1).rev() {
        result = result * x + coeffs[i];
    }
    
    result
}

/// Batch matrix-vector multiplication for field elements
pub fn batch_matrix_vector_mul_jet(context: &mut Context, subject: Noun) -> Result {
    let sam = slot(subject, 6)?;
    let matrix = slot(sam, 2)?;  // List of rows
    let vector = slot(sam, 3)?;
    
    let vec_elems: Vec<Belt> = parse_belt_list(vector)?;
    
    // Parse matrix rows
    let matrix_rows: Vec<Vec<Belt>> = HoonList::try_from(matrix)?
        .into_iter()
        .map(|row| parse_belt_list(row))
        .collect::<Result<Vec<_>>>()?;
    
    // Check dimensions
    for row in &matrix_rows {
        if row.len() != vec_elems.len() {
            return jet_err();
        }
    }
    
    // Parallel matrix-vector multiplication
    let results: Vec<Belt> = matrix_rows
        .par_iter()
        .map(|row| {
            row.iter()
                .zip(vec_elems.iter())
                .map(|(&a, &b)| a * b)
                .fold(Belt::ZERO, |acc, x| acc + x)
        })
        .collect();
    
    belt_vec_to_noun_list(context, &results)
}