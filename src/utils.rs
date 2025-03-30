use rand::{self, Rng};

pub fn efraimidis_spirakis_sample(
    weights: &[f32],
    count: usize,
    rng: &mut rand::rngs::ThreadRng,
) -> Vec<usize> {
    let keys = rng.random_iter().take(count).collect::<Vec<f32>>();
    let mut results= keys.iter()
        .zip(weights.iter())
        .zip(0..weights.len())
        .map(|((key, weight), idx)| (-key.ln() / weight, idx))
        .collect::<Vec<_>>();
    results.sort_by(|a, b| a.partial_cmp(b).unwrap());
    results.iter().take(count).map(|(_, idx)| *idx).collect()
}