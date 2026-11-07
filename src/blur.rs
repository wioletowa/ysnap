// Размытие: три прохода прямоугольником по каждой оси ≈ гаусс. Общее для оверлея (лёгкое
// размытие, snapdraw) и тени снимка окна (winshot). Край не темнеет: в окне считается только то,
// что внутри картинки.

/// один проход радиуса r вдоль строк (horiz) или столбцов
fn pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horiz: bool) {
    let (n, lines) = if horiz { (w, h) } else { (h, w) };
    let at = |l: usize, i: usize| if horiz { l * w + i } else { i * w + l };
    for l in 0..lines {
        let (mut acc, mut cnt) = (0f32, 0f32);
        for i in 0..r.min(n) {
            acc += src[at(l, i)];
            cnt += 1.0;
        }
        for i in 0..n {
            if i + r < n {
                acc += src[at(l, i + r)];
                cnt += 1.0;
            }
            if i > r {
                acc -= src[at(l, i - r - 1)];
                cnt -= 1.0;
            }
            dst[at(l, i)] = acc / cnt;
        }
    }
}

/// канал w×h на месте, радиус прямоугольника r
pub fn box3(a: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0f32; a.len()];
    for _ in 0..3 {
        pass(a, &mut tmp, w, h, r, true);
        pass(&tmp, a, w, h, r, false);
    }
}

/// радиус прямоугольника для трёх проходов, близких к гауссу с этой сигмой
pub fn radius(sigma: f32) -> usize {
    if sigma < 0.5 {
        return 0;
    }
    (((12.0 * sigma * sigma / 3.0 + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_stays_flat_and_edges_do_not_darken() {
        let mut a = vec![10.0; 7 * 5];
        box3(&mut a, 7, 5, 2);
        assert!(a.iter().all(|v| (v - 10.0).abs() < 1e-4));
        let mut b = vec![0.0; 9];
        b[4] = 9.0;
        box3(&mut b, 9, 1, 1);
        assert!((b.iter().sum::<f32>() - 9.0).abs() < 1.5 && b[4] > b[0]);
    }
}
