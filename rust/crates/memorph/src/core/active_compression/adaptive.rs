use std::collections::HashSet;

pub(super) fn adaptive_keep_count(items: &[String], min_k: usize, max_k: usize) -> usize {
    let total = items.len();
    if total == 0 || max_k == 0 {
        return 0;
    }

    let effective_max = max_k.min(total);
    if total <= 8 {
        return total.min(effective_max);
    }

    let unique_count = count_unique_simhash(items, 3);
    if unique_count <= 3 {
        return min_k.max(unique_count).min(effective_max);
    }

    let curve = unique_bigram_curve(items);
    let diversity_ratio = unique_count as f64 / total as f64;
    let mut keep = match find_knee(&curve) {
        Some(knee) if diversity_ratio > 0.7 => {
            let floor = (total as f64 * (0.3 + 0.7 * diversity_ratio)) as usize;
            knee.max(floor)
        }
        Some(knee) => knee,
        None => (total as f64 * (0.3 + 0.7 * diversity_ratio)) as usize,
    };

    keep = keep.max(min_k).min(effective_max);
    validate_diversity_floor(items, keep, effective_max)
}

fn unique_bigram_curve(items: &[String]) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut curve = Vec::with_capacity(items.len());

    for item in items {
        let lower = item.to_ascii_lowercase();
        let words = lower.split_whitespace().collect::<Vec<_>>();
        if words.len() < 2 {
            seen.insert((
                words.first().copied().unwrap_or("").to_string(),
                String::new(),
            ));
        } else {
            for pair in words.windows(2) {
                seen.insert((pair[0].to_string(), pair[1].to_string()));
            }
        }
        curve.push(seen.len());
    }

    curve
}

fn find_knee(curve: &[usize]) -> Option<usize> {
    if curve.len() < 3 {
        return None;
    }

    let y_min = curve[0] as f64;
    let y_max = *curve.last()? as f64;
    if (y_max - y_min).abs() < f64::EPSILON {
        return Some(1);
    }

    let x_range = (curve.len() - 1) as f64;
    let y_range = y_max - y_min;
    let mut best_diff = -1.0;
    let mut best_index = None;

    for (index, value) in curve.iter().enumerate() {
        let x_norm = index as f64 / x_range;
        let y_norm = (*value as f64 - y_min) / y_range;
        let diff = y_norm - x_norm;
        if diff > best_diff {
            best_diff = diff;
            best_index = Some(index);
        }
    }

    if best_diff < 0.05 {
        None
    } else {
        best_index.map(|index| index + 1)
    }
}

fn count_unique_simhash(items: &[String], max_distance: u32) -> usize {
    let mut clusters = Vec::<u64>::new();
    'items: for item in items {
        let hash = simhash(item);
        for existing in &clusters {
            if (hash ^ *existing).count_ones() <= max_distance {
                continue 'items;
            }
        }
        clusters.push(hash);
    }
    clusters.len()
}

fn simhash(text: &str) -> u64 {
    let lower = text.to_ascii_lowercase();
    let chars = lower.chars().collect::<Vec<_>>();
    let gram_count = if chars.len() <= 3 { 1 } else { chars.len() - 3 };
    let mut votes = [0i32; 64];

    for index in 0..gram_count {
        let gram = chars.iter().skip(index).take(4).collect::<String>();
        let hash = fnv1a64(gram.as_bytes());
        for (bit, vote) in votes.iter_mut().enumerate() {
            if (hash >> bit) & 1 == 1 {
                *vote += 1;
            } else {
                *vote -= 1;
            }
        }
    }

    let mut out = 0u64;
    for (bit, vote) in votes.iter().enumerate() {
        if *vote > 0 {
            out |= 1 << bit;
        }
    }
    out
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn validate_diversity_floor(items: &[String], keep: usize, effective_max: usize) -> usize {
    if keep >= effective_max || items.is_empty() {
        return keep;
    }

    let all_ratio = unique_word_ratio(items);
    let kept_ratio = unique_word_ratio(&items[..keep.min(items.len())]);
    if all_ratio > 0.0 && kept_ratio + 0.15 < all_ratio {
        (keep + keep.div_ceil(5)).min(effective_max)
    } else {
        keep
    }
}

fn unique_word_ratio(items: &[String]) -> f64 {
    let mut unique = HashSet::new();
    let mut total = 0usize;

    for item in items {
        for word in item.split_whitespace() {
            total += 1;
            unique.insert(word.to_ascii_lowercase());
        }
    }

    if total == 0 {
        0.0
    } else {
        unique.len() as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_tiny_sets_whole() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        assert_eq!(adaptive_keep_count(&items, 1, 10), 3);
    }

    #[test]
    fn collapses_highly_redundant_sets() {
        let items = (0..30)
            .map(|_| "same repeated warning line".to_string())
            .collect::<Vec<_>>();

        assert!(adaptive_keep_count(&items, 2, 20) <= 3);
    }

    #[test]
    fn keeps_more_for_diverse_sets() {
        let items = (0..30)
            .map(|idx| format!("src/file_{idx}.rs unique symbol {idx}"))
            .collect::<Vec<_>>();

        assert!(adaptive_keep_count(&items, 2, 20) > 10);
    }
}
