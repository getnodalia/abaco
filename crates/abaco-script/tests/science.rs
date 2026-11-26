// SPDX-License-Identifier: MIT OR Apache-2.0
//! Signals, interpolation, statistics and special functions against analytic results.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::{Interp, Value};

fn run(src: &str) -> Interp {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i
}

fn real(src: &str) -> Vec<f64> {
    let i = run(src);
    i.get("ans").unwrap().to_real().unwrap().d
}

fn scalar(src: &str) -> f64 {
    let v = real(src);
    assert_eq!(v.len(), 1, "{src} gave {v:?}");
    v[0]
}

fn near(a: f64, b: f64, tol: f64) {
    assert!(
        (a - b).abs() <= tol * b.abs().max(1.0),
        "{a} vs {b} (tol {tol})"
    );
}

fn all_near(a: &[f64], b: &[f64], tol: f64) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        near(*x, *y, tol);
    }
}

fn fails(src: &str) -> String {
    let mut i = Interp::new();
    match i.run(src) {
        Ok(()) => panic!("`{src}` should have failed"),
        Err(e) => e.message,
    }
}

#[test]
fn fft_of_a_short_signal_and_back() {
    let i = run("X = fft([1 2 3 4]); r = real(X); m = imag(X); y = ifft(X);");
    all_near(
        &i.get("r").unwrap().to_real().unwrap().d,
        &[10.0, -2.0, -2.0, -2.0],
        1e-12,
    );
    all_near(
        &i.get("m").unwrap().to_real().unwrap().d,
        &[0.0, 2.0, 0.0, -2.0],
        1e-12,
    );
    all_near(
        &i.get("y").unwrap().to_real().unwrap().d,
        &[1.0, 2.0, 3.0, 4.0],
        1e-12,
    );
    // odd lengths and padding
    near(
        scalar("ax = abs(fft([1 1 1 1 1])); ans = ax(1)"),
        5.0,
        1e-12,
    );
    assert_eq!(real("size(fft([1 2 3], 8))"), vec![1.0, 8.0]);
    // a tone lands in one bin
    near(
        scalar("x = cos(2*pi*3*(0:31)/32); ax = abs(fft(x)); ans = ax(4)"),
        16.0,
        1e-10,
    );
}

#[test]
fn fft_works_along_columns_and_rows_and_in_two_dimensions() {
    all_near(&real("abs(fft([1 2; 3 4]))"), &[4.0, 2.0, 6.0, 2.0], 1e-12);
    all_near(
        &real("abs(fft([1 2; 3 4], [], 2))"),
        &[3.0, 7.0, 1.0, 1.0],
        1e-12,
    );
    near(
        scalar("A = magic(4); X = fft2(A); real(X(1,1))"),
        136.0,
        1e-12,
    );
    all_near(
        &real("A = magic(3); real(ifft2(fft2(A)))"),
        &real("magic(3)"),
        1e-12,
    );
}

#[test]
fn shifts_and_nextpow2() {
    all_near(
        &real("fftshift([1 2 3 4 5])"),
        &[4.0, 5.0, 1.0, 2.0, 3.0],
        0.0,
    );
    all_near(
        &real("ifftshift(fftshift([1 2 3 4 5]))"),
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        0.0,
    );
    assert_eq!(real("nextpow2([1 5 100 1024])"), vec![0.0, 3.0, 7.0, 10.0]);
}

#[test]
fn filters_match_hand_computation() {
    all_near(
        &real("filter(ones(1,3)/3, 1, [3 6 9 12])"),
        &[1.0, 3.0, 6.0, 9.0],
        1e-14,
    );
    all_near(
        &real("filter(1, [1 -0.5], [1 0 0 0])"),
        &[1.0, 0.5, 0.25, 0.125],
        1e-14,
    );
    assert!(fails("filter(1, [0 1], [1 2 3])").contains("denominator"));
    // zero-phase filtering has no lag: a constant stays constant
    all_near(
        &real("[b,a] = butter(2, 0.3); y = filtfilt(b, a, 5*ones(1,40)); ans = y(1:3)"),
        &[5.0, 5.0, 5.0],
        1e-6,
    );
}

#[test]
fn butterworth_design_matches_reference_coefficients() {
    // scipy.signal.butter, the standard reference for these designs.
    let b = real("[b,a] = butter(2, 0.1); ans = b");
    all_near(
        &b,
        &[
            0.020_083_365_564_211_236,
            0.040_166_731_128_422_47,
            0.020_083_365_564_211_236,
        ],
        1e-9,
    );
    let a = real("[b,a] = butter(2, 0.1); ans = a");
    all_near(
        &a,
        &[1.0, -1.561_018_075_800_718_2, 0.641_351_538_057_563_2],
        1e-9,
    );
    let b = real("[b,a] = butter(2, 0.2); ans = b");
    all_near(
        &b,
        &[
            0.067_455_273_889_071_9,
            0.134_910_547_778_143_8,
            0.067_455_273_889_071_9,
        ],
        1e-9,
    );
    let a = real("[b,a] = butter(2, 0.2); ans = a");
    all_near(
        &a,
        &[1.0, -1.142_980_502_539_901_1, 0.412_801_598_096_189_2],
        1e-9,
    );
}

#[test]
fn filter_gains_at_dc_nyquist_and_cutoff() {
    let gain = |design: &str, w: &str| {
        scalar(&format!(
            "{design}; h = freqz(b, a, [{w} {w}]); ans = abs(h(1))"
        ))
    };
    // lowpass: unit at DC, nothing at Nyquist, -3 dB at the cutoff
    near(gain("[b,a] = butter(4, 0.3)", "0"), 1.0, 1e-9);
    assert!(gain("[b,a] = butter(4, 0.3)", "pi") < 1e-4);
    near(
        gain("[b,a] = butter(4, 0.3)", "0.3*pi"),
        std::f64::consts::FRAC_1_SQRT_2,
        1e-9,
    );
    // highpass
    assert!(gain("[b,a] = butter(3, 0.4, 'high')", "0") < 1e-9);
    near(gain("[b,a] = butter(3, 0.4, 'high')", "pi"), 1.0, 1e-9);
    // bandpass and bandstop edges are at -3 dB
    near(
        gain("[b,a] = butter(3, [0.2 0.5])", "0.2*pi"),
        std::f64::consts::FRAC_1_SQRT_2,
        1e-8,
    );
    near(
        gain("[b,a] = butter(3, [0.2 0.5])", "0.5*pi"),
        std::f64::consts::FRAC_1_SQRT_2,
        1e-8,
    );
    near(gain("[b,a] = butter(3, [0.2 0.5], 'stop')", "0"), 1.0, 1e-9);
    assert!(gain("[b,a] = butter(3, [0.2 0.5], 'stop')", "sqrt(0.2*0.5)*pi") < 0.05);
    // Chebyshev type I: the passband stays within the ripple
    let lo = scalar(
        "[b,a] = cheby1(4, 1, 0.4); h = abs(freqz(b, a, linspace(0, 0.4*pi, 200))); ans = min(h)",
    );
    assert!(
        lo >= 10f64.powf(-1.0 / 20.0) - 1e-6,
        "ripple exceeded: {lo}"
    );
    near(
        scalar("[b,a] = cheby1(4, 1, 0.4); ans = max(abs(freqz(b, a, linspace(0, 0.4*pi, 200))))"),
        1.0,
        1e-6,
    );
}

#[test]
fn fir_design_and_windows() {
    all_near(&real("hamming(5)"), &[0.08, 0.54, 1.0, 0.54, 0.08], 1e-12);
    all_near(&real("hann(5)"), &[0.0, 0.5, 1.0, 0.5, 0.0], 1e-12);
    all_near(&real("bartlett(5)"), &[0.0, 0.5, 1.0, 0.5, 0.0], 1e-12);
    near(scalar("max(kaiser(9, 5))"), 1.0, 1e-12);
    // symmetric, unit DC gain
    let h = real("fir1(20, 0.4)");
    assert_eq!(h.len(), 21);
    for k in 0..10 {
        near(h[k], h[20 - k], 1e-12);
    }
    near(h.iter().sum::<f64>(), 1.0, 1e-12);
    near(
        scalar("h = fir1(30, 0.4, 'high'); g = abs(freqz(h, 1, [pi pi])); ans = g(1)"),
        1.0,
        1e-9,
    );
    assert!(fails("fir1(21, 0.4, 'high')").contains("even"));
}

#[test]
fn correlation_peaks_and_trends() {
    all_near(
        &real("xcorr([1 2 3], [1 2 3])"),
        &[3.0, 8.0, 14.0, 8.0, 3.0],
        1e-10,
    );
    all_near(
        &real("[c, l] = xcorr([1 2 3]); ans = l"),
        &[-2.0, -1.0, 0.0, 1.0, 2.0],
        0.0,
    );
    all_near(
        &real("[p, l] = findpeaks([1 3 1 5 1]); ans = p"),
        &[3.0, 5.0],
        0.0,
    );
    all_near(
        &real("[p, l] = findpeaks([1 3 1 5 1]); ans = l"),
        &[2.0, 4.0],
        0.0,
    );
    all_near(
        &real("[p, l] = findpeaks([1 3 1 5 1], 'MinPeakHeight', 4); ans = l"),
        &[4.0],
        0.0,
    );
    all_near(&real("detrend([1 2 3 4])"), &[0.0, 0.0, 0.0, 0.0], 1e-12);
    all_near(&real("unwrap([0 2*pi 4*pi])"), &[0.0, 0.0, 0.0], 1e-12);
    near(scalar("sinc(0.5)"), 2.0 / std::f64::consts::PI, 1e-14);
}

#[test]
fn interpolation_methods() {
    near(scalar("interp1([1 2 3], [10 20 30], 2.5)"), 25.0, 1e-14);
    assert!(scalar("interp1([1 2 3], [10 20 30], 5)").is_nan());
    near(
        scalar("interp1([1 2 3], [10 20 30], 5, 'linear', 'extrap')"),
        50.0,
        1e-12,
    );
    near(
        scalar("interp1([1 2 3], [10 20 30], 5, 'linear', -1)"),
        -1.0,
        0.0,
    );
    near(
        scalar("interp1([1 2 3], [10 20 30], 2.4, 'nearest')"),
        20.0,
        0.0,
    );
    near(
        scalar("interp1([1 2 3], [10 20 30], 2.4, 'previous')"),
        20.0,
        0.0,
    );
    near(
        scalar("interp1([1 2 3], [10 20 30], 2.4, 'next')"),
        30.0,
        0.0,
    );
    // values only: x = 1:n
    near(scalar("interp1([10 20 30], 1.5)"), 15.0, 1e-14);
    // a cubic is reproduced exactly by a not-a-knot spline
    near(scalar("x = 0:0.7:4.2; y = 2 - x + 0.5*x.^2 - 0.25*x.^3; ans = interp1(x, y, 1.9, 'spline') - (2 - 1.9 + 0.5*1.9^2 - 0.25*1.9^3)"), 0.0, 1e-10);
    near(scalar("spline(0:4, (0:4).^3, 2.5)"), 15.625, 1e-12);
    // pchip does not overshoot a step
    assert!(
        scalar("max(interp1([0 1 2 3 4], [0 0 1 1 1], linspace(0, 4, 100), 'pchip'))")
            <= 1.0 + 1e-12
    );
    // the piecewise polynomial can be evaluated later
    near(
        scalar("pp = spline(0:4, (0:4).^3); ans = ppval(pp, 3.5)"),
        42.875,
        1e-12,
    );
    near(
        scalar("pp = mkpp([0 1 2], [1 0; 2 1]); ans = ppval(pp, 1.5)"),
        2.0,
        1e-14,
    );
    // matrix of columns
    all_near(
        &real("interp1([1 2 3], [10 100; 20 200; 30 300], 1.5)"),
        &[15.0, 150.0],
        1e-14,
    );
    assert!(fails("interp1([1 1 2], [1 2 3], 1.5)").contains("distinct"));
}

#[test]
fn bilinear_interpolation() {
    near(scalar("interp2([1 2; 3 4], 1.5, 1.5)"), 2.5, 1e-14);
    near(
        scalar("[X, Y] = meshgrid(0:2, 0:2); V = X + 10*Y; ans = interp2(X, Y, V, 0.5, 1.25)"),
        13.0,
        1e-13,
    );
    assert!(scalar("interp2([1 2; 3 4], 5, 1)").is_nan());
}

#[test]
fn special_functions_against_reference_values() {
    near(scalar("erfinv(erf(0.3))"), 0.3, 1e-12);
    near(scalar("erfcinv(erfc(2.0))"), 2.0, 1e-10);
    near(scalar("gammainc(1, 2)"), 1.0 - 2.0 * (-1.0f64).exp(), 1e-13);
    near(
        scalar("gammainc(1, 2, 'upper')"),
        2.0 * (-1.0f64).exp(),
        1e-13,
    );
    near(scalar("betainc(0.5, 2, 3)"), 0.6875, 1e-13);
    near(scalar("beta(2, 3)"), 1.0 / 12.0, 1e-13);
}

#[test]
fn distributions_against_tables() {
    near(scalar("normcdf(0)"), 0.5, 1e-15);
    near(scalar("normcdf(1.96)"), 0.975_002_104_851_78, 1e-10);
    near(scalar("norminv(0.975)"), 1.959_963_984_540_054, 1e-9);
    near(scalar("normpdf(0)"), 0.398_942_280_401_432_7, 1e-14);
    near(scalar("normcdf(12, 10, 2)"), 0.841_344_746_068_543, 1e-10);
    near(scalar("chi2cdf(3.841458820694124, 1)"), 0.95, 1e-9);
    near(scalar("tcdf(2.228138851986274, 10)"), 0.975, 1e-9);
    near(scalar("tcdf(-2.228138851986274, 10)"), 0.025, 1e-9);
    near(scalar("binopdf(3, 10, 0.5)"), 120.0 / 1024.0, 1e-13);
    near(scalar("binocdf(3, 10, 0.5)"), 176.0 / 1024.0, 1e-12);
    near(scalar("poisspdf(2, 3)"), (-3.0f64).exp() * 4.5, 1e-13);
    near(
        scalar("poisscdf(2, 3)"),
        (-3.0f64).exp() * (1.0 + 3.0 + 4.5),
        1e-12,
    );
    near(scalar("expcdf(2, 2)"), 1.0 - (-1.0f64).exp(), 1e-14);
    near(scalar("fcdf(1, 5, 5)"), 0.5, 1e-12);
    near(scalar("unifcdf(0.3)"), 0.3, 1e-15);
    assert!(scalar("normcdf(0, 0, -1)").is_nan());
}

#[test]
fn descriptive_statistics() {
    near(scalar("skewness([1 2 3 4 5])"), 0.0, 1e-14);
    assert!(scalar("skewness([1 1 1 10])") > 0.0);
    near(scalar("kurtosis([1 2 3 4 5])"), 1.7, 1e-14);
    near(scalar("quantile([1 2 3 4 5], 0.5)"), 3.0, 1e-14);
    near(scalar("iqr([1 2 3 4 5 6 7 8])"), 4.0, 1e-12);
    near(scalar("rms([3 4])"), (12.5f64).sqrt(), 1e-14);
    near(scalar("mad([1 2 3 4 100], 1)"), 1.0, 1e-14);
    all_near(&real("zscore([1 2 3])"), &[-1.0, 0.0, 1.0], 1e-14);
    all_near(&real("rescale([2 4 6])"), &[0.0, 0.5, 1.0], 1e-14);
    all_near(
        &real("[lo, hi] = bounds([3 1 4 1 5]); ans = [lo hi]"),
        &[1.0, 5.0],
        0.0,
    );
    near(scalar("corr([1 2 3 4]', [2 4 6 8]')"), 1.0, 1e-14);
    all_near(
        &real("movmean([1 2 3 4 5], 3)"),
        &[1.5, 2.0, 3.0, 4.0, 4.5],
        1e-14,
    );
    all_near(
        &real("movmax([1 5 2 4 3], 3)"),
        &[5.0, 5.0, 5.0, 4.0, 4.0],
        0.0,
    );
    all_near(
        &real("[n, e] = histcounts([1 2 2 3 3 3], 3); ans = n"),
        &[1.0, 2.0, 3.0],
        0.0,
    );
    all_near(
        &real("n = histcounts([0.5 1.5 2.5 9], [0 1 2 3]); ans = n"),
        &[1.0, 1.0, 1.0],
        0.0,
    );
}

#[test]
fn normal_random_numbers_have_the_requested_moments() {
    let i = run("rng(1); x = normrnd(10, 2, 20000, 1); m = mean(x); s = std(x);");
    near(i.get("m").unwrap().scalar_f64("m").unwrap(), 10.0, 0.02);
    near(i.get("s").unwrap().scalar_f64("s").unwrap(), 2.0, 0.03);
}

#[test]
fn hostile_arguments_are_errors_not_crashes() {
    for src in [
        "fft('a', -1)",
        "fft(ones(1,3), 1e12)",
        "butter(0, 0.5)",
        "butter(2, 1.5)",
        "butter(2, [0.5 0.2])",
        "cheby1(3, -1, 0.5)",
        "fir1(0, 0.5)",
        "interp1(1, 1, 1)",
        "interp1([1 2], [1 2 3], 1)",
        "spline(1:3, 1:2)",
        "ppval(5, 1)",
        "mkpp([0 1 1], [1; 2])",
        "histcounts([1 2], [3 2 1])",
        "movmean(1:3, 0)",
        "xcorr([])",
        "freqz(1, 0, 4)",
        "filtfilt([1 1], 1, [1 2 3])",
    ] {
        let mut i = Interp::new();
        let r = i.run(src);
        assert!(
            r.is_err() || src.starts_with("freqz"),
            "`{src}` should be refused"
        );
    }
    // anything below must not panic, whatever it returns
    let mut i = Interp::new();
    let _ = i.run("freqz(1, 0, 4)");
    let _: Option<&Value> = i.get("ans");
}
