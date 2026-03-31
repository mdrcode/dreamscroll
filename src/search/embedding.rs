use std::marker::PhantomData;
use serde::{Deserialize, Serialize};

/// Marker type for raw (not guaranteed normalized) embeddings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raw;

/// Marker type for L2-normalized embeddings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit;

/// Strongly typed embedding wrapper.
///
/// `S` is a type-state marker that indicates whether values are known to be
/// normalized (`Unit`) or not (`Raw`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Embedding<T = f32, S = Raw> {
    values: Vec<T>,
    _state: PhantomData<S>,
}

impl Embedding<f32, Raw> {
    /// Creates a raw embedding after validating it is non-empty and finite.
    pub fn from_vec(values: Vec<f32>) -> anyhow::Result<Self> {
        validate_values(&values)?;
        Ok(Self {
            values,
            _state: PhantomData,
        })
    }

    /// Creates a unit embedding by normalizing an owned vector in place.
    pub fn from_vec_normalizing(mut values: Vec<f32>) -> anyhow::Result<Embedding<f32, Unit>> {
        validate_values(&values)?;
        normalize_l2_in_place(&mut values)?;
        Ok(Embedding {
            values,
            _state: PhantomData,
        })
    }

    /// Converts this raw embedding into a unit embedding via L2 normalization.
    pub fn normalize(mut self) -> anyhow::Result<Embedding<f32, Unit>> {
        normalize_l2_in_place(&mut self.values)?;
        Ok(Embedding {
            values: self.values,
            _state: PhantomData,
        })
    }
}

impl<S> Embedding<f32, S> {
    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn into_vec(self) -> Vec<f32> {
        self.values
    }

    /// Returns a normalized clone of this embedding.
    pub fn normalized(&self) -> anyhow::Result<Embedding<f32, Unit>> {
        Embedding::<f32, Raw>::from_vec(self.values.clone())?.normalize()
    }
}

fn validate_values(values: &[f32]) -> anyhow::Result<()> {
    if values.is_empty() {
        anyhow::bail!("Embedding is empty");
    }

    if values.iter().any(|v| !v.is_finite()) {
        anyhow::bail!("Embedding contains non-finite values (NaN or +/-Inf)");
    }

    Ok(())
}

fn normalize_l2_in_place(values: &mut [f32]) -> anyhow::Result<()> {
    let norm_sq: f64 = values
        .iter()
        .map(|v| {
            let x = *v as f64;
            x * x
        })
        .sum();

    if !norm_sq.is_finite() || norm_sq <= 0.0 {
        anyhow::bail!("Embedding has zero or invalid L2 norm; cannot normalize");
    }

    let norm = norm_sq.sqrt() as f32;
    for v in values.iter_mut() {
        *v /= norm;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l2_norm(values: &[f32]) -> f32 {
        values.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    #[test]
    fn from_vec_rejects_empty() {
        let err = Embedding::<f32, Raw>::from_vec(vec![]).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn from_vec_rejects_non_finite_values() {
        let err = Embedding::<f32, Raw>::from_vec(vec![1.0, f32::NAN]).unwrap_err();
        assert!(err.to_string().contains("non-finite"));
    }

    #[test]
    fn from_vec_accepts_finite_non_empty_values() {
        let emb = Embedding::<f32, Raw>::from_vec(vec![1.0, 2.0, 3.0]).unwrap();
        assert_eq!(emb.len(), 3);
        assert!(!emb.is_empty());
    }

    #[test]
    fn normalize_produces_unit_vector() {
        let emb = Embedding::<f32, Raw>::from_vec(vec![3.0, 4.0]).unwrap();
        let unit = emb.normalize().unwrap();
        let norm = l2_norm(unit.as_slice());
        assert!((norm - 1.0).abs() < 1e-6, "norm={} expected ~1", norm);
    }

    #[test]
    fn from_vec_normalizing_produces_unit_vector() {
        let unit = Embedding::<f32, Raw>::from_vec_normalizing(vec![10.0, 0.0]).unwrap();
        let norm = l2_norm(unit.as_slice());
        assert!((norm - 1.0).abs() < 1e-6, "norm={} expected ~1", norm);
    }

    #[test]
    fn normalize_rejects_zero_vector() {
        let emb = Embedding::<f32, Raw>::from_vec(vec![0.0, 0.0, 0.0]).unwrap();
        let err = emb.normalize().unwrap_err();
        assert!(err.to_string().contains("cannot normalize"));
    }

    #[test]
    fn into_vec_round_trip() {
        let input = vec![1.5, -2.5, 3.25];
        let emb = Embedding::<f32, Raw>::from_vec(input.clone()).unwrap();
        let output = emb.into_vec();
        assert_eq!(input, output);
    }
}
