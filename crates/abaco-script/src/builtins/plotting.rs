// SPDX-License-Identifier: MIT OR Apache-2.0
//! Plotting commands. They record what was asked for in `Interp::figures` and announce a
//! change with [`Event::Figure`]; the host draws it.

use super::{arity, is_flag, num, one, real_arg, str_arg, Bf, Out};
use crate::error::{err, Result};
use crate::interp::{Event, Interp};
use crate::plot::{Axes, Figure, Kind, Series, Surface};
use crate::value::{Arr, Value};

/// Most series one command may add (a matrix of columns plots one series per column).
const MAX_SERIES: usize = 2_000;
/// Most cells in a subplot grid.
const MAX_CELLS: usize = 64;

fn changed(i: &mut Interp) {
    let n = i.figures.current;
    i.emit(Event::Figure(n));
}

/// Property names accepted (and mostly ignored) after the data of a plot command.
fn is_property(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "displayname"
            | "linewidth"
            | "color"
            | "marker"
            | "linestyle"
            | "markersize"
            | "markerfacecolor"
            | "markeredgecolor"
            | "facecolor"
            | "edgecolor"
            | "facealpha"
            | "filled"
    )
}

/// The `x` and `y` pieces for each series of one `(x, y)` pair; matrices plot by column.
fn split_series(
    x: Option<&Arr<f64>>,
    y: &Arr<f64>,
    name: &str,
) -> Result<Vec<(Vec<f64>, Vec<f64>)>> {
    let cols: Vec<Vec<f64>> = if y.is_vector() || y.is_empty() {
        vec![y.d.clone()]
    } else {
        (0..y.c).map(|j| y.column(j)).collect()
    };
    if cols.len() > MAX_SERIES {
        return err(
            "Abaco:limit",
            format!(
                "`{name}`: {} series is more than the limit of {MAX_SERIES}",
                cols.len()
            ),
        );
    }
    let mut out = Vec::with_capacity(cols.len());
    for (j, ycol) in cols.into_iter().enumerate() {
        let xs: Vec<f64> = match x {
            None => (1..=ycol.len()).map(|k| k as f64).collect(),
            Some(x) if x.is_vector() || x.is_empty() => x.d.clone(),
            Some(x) if x.r == y.r && x.c == y.c => x.column(j),
            Some(x) => x.d.clone(),
        };
        if xs.len() != ycol.len() {
            return err(
                "Abaco:size",
                format!(
                    "`{name}`: x has {} values but y has {}; they must match",
                    xs.len(),
                    ycol.len()
                ),
            );
        }
        out.push((xs, ycol));
    }
    Ok(out)
}

struct Parsed {
    series: Vec<Series>,
}

/// Reads `plot(x1, y1, spec1, x2, y2, ..., 'Prop', value)` style arguments.
fn parse_xy(args: &[Value], kind: Kind, name: &str) -> Result<Parsed> {
    let mut series = Vec::new();
    let mut label = String::new();
    let mut k = 0;
    let mut pending: Vec<(Vec<f64>, Vec<f64>, String)> = Vec::new();
    while k < args.len() {
        match &args[k] {
            Value::Str(s) if is_property(s) && k + 1 < args.len() => {
                if s.eq_ignore_ascii_case("displayname") {
                    if let Value::Str(l) = &args[k + 1] {
                        label = l.clone();
                    }
                }
                k += 2;
            }
            Value::Str(s) => {
                return err(
                    "Abaco:argument",
                    format!("`{name}`: unexpected text `{s}`; a line style such as 'r--o' must follow the data it styles"),
                );
            }
            Value::Cplx(z) if !matches!(args.get(k + 1), Some(v) if !matches!(v, Value::Str(_))) => {
                let re: Vec<f64> = z.d.iter().map(|c| c.re).collect();
                let im: Vec<f64> = z.d.iter().map(|c| c.im).collect();
                let spec = spec_at(args, k + 1);
                k += 1 + usize::from(!spec.is_empty());
                pending.push((re, im, spec));
            }
            first => {
                let a = first.to_real()?;
                let second = args.get(k + 1).filter(|v| !matches!(v, Value::Str(_)));
                match second {
                    Some(v) => {
                        let b = v.to_real()?;
                        let spec = spec_at(args, k + 2);
                        for (xs, ys) in split_series(Some(&a), &b, name)? {
                            pending.push((xs, ys, spec.clone()));
                        }
                        k += 2 + usize::from(!spec.is_empty());
                    }
                    None => {
                        let spec = spec_at(args, k + 1);
                        for (xs, ys) in split_series(None, &a, name)? {
                            pending.push((xs, ys, spec.clone()));
                        }
                        k += 1 + usize::from(!spec.is_empty());
                    }
                }
            }
        }
        if pending.len() > MAX_SERIES {
            return err(
                "Abaco:limit",
                format!("`{name}`: more than {MAX_SERIES} series in one call"),
            );
        }
    }
    if pending.is_empty() {
        return err("Abaco:arity", format!("`{name}` needs data to draw"));
    }
    for (x, y, spec) in pending {
        series.push(Series {
            kind,
            x,
            y,
            z: Vec::new(),
            spec,
            label: String::new(),
        });
    }
    if !label.is_empty() {
        if let Some(last) = series.last_mut() {
            last.label = label;
        }
    }
    Ok(Parsed { series })
}

/// The line-spec text at `args[k]`, when it is one (not a property name).
fn spec_at(args: &[Value], k: usize) -> String {
    match args.get(k) {
        Some(Value::Str(s)) if !is_property(s) => s.clone(),
        _ => String::new(),
    }
}

/// The axes commands draw into, cleared first unless `hold on`.
fn target(i: &mut Interp) -> &mut Axes {
    let fig = i.figures.cur();
    let hold = fig.hold;
    let ax = fig.ax();
    if !hold {
        *ax = Axes::default();
    }
    ax
}

fn draw_series(i: &mut Interp, parsed: Parsed, xlog: bool, ylog: bool) -> Out {
    let ax = target(i);
    ax.series.extend(parsed.series);
    ax.xlog |= xlog;
    ax.ylog |= ylog;
    changed(i);
    Ok(vec![])
}

fn plot_kind(i: &mut Interp, a: &[Value], kind: Kind, name: &str, xlog: bool, ylog: bool) -> Out {
    let parsed = parse_xy(a, kind, name)?;
    draw_series(i, parsed, xlog, ylog)
}

fn f_plot(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Line, "plot", false, false)
}

fn f_semilogx(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Line, "semilogx", true, false)
}

fn f_semilogy(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Line, "semilogy", false, true)
}

fn f_loglog(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Line, "loglog", true, true)
}

fn f_scatter(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, usize::MAX, "scatter")?;
    // scatter(x, y, size, color): size and colour are accepted and ignored.
    let data: Vec<Value> = a.iter().take(2).cloned().collect();
    let spec = a.iter().skip(2).find_map(|v| match v {
        Value::Str(s) if !is_property(s) && s != "filled" => Some(Value::text(s.clone())),
        _ => None,
    });
    let mut args = data;
    args.extend(spec);
    plot_kind(i, &args, Kind::Scatter, "scatter", false, false)
}

fn f_stem(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Stem, "stem", false, false)
}

fn f_stairs(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Stairs, "stairs", false, false)
}

fn f_bar(i: &mut Interp, a: &[Value], _: usize) -> Out {
    plot_kind(i, a, Kind::Bar, "bar", false, false)
}

fn f_plot3(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 4, "plot3")?;
    let x = real_arg(a, 0, "plot3")?;
    let y = real_arg(a, 1, "plot3")?;
    let z = real_arg(a, 2, "plot3")?;
    if x.len() != y.len() || x.len() != z.len() {
        return err(
            "Abaco:size",
            format!(
                "`plot3`: x, y and z must have the same number of values, got {}, {} and {}",
                x.len(),
                y.len(),
                z.len()
            ),
        );
    }
    let spec = super::str_arg(a, 3, "plot3")
        .map(String::from)
        .unwrap_or_default();
    let ax = target(i);
    ax.series.push(Series {
        kind: Kind::Line3,
        x: x.d,
        y: y.d,
        z: z.d,
        spec,
        label: String::new(),
    });
    changed(i);
    Ok(vec![])
}

fn f_hist(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 1, 2, "hist")?;
    let data: Vec<f64> = real_arg(a, 0, "hist")?
        .d
        .into_iter()
        .filter(|x| x.is_finite())
        .collect();
    let centers: Vec<f64> = match a.get(1) {
        Some(v) => {
            let b = v.to_real()?;
            if b.is_scalar() {
                bin_centers(&data, count_arg(b.d[0])?)
            } else {
                b.d
            }
        }
        None => bin_centers(&data, 10),
    };
    let counts = bin_counts(&data, &centers);
    if nargout > 0 {
        return Ok(vec![Value::row(counts), Value::row(centers)]);
    }
    let ax = target(i);
    ax.series.push(Series {
        kind: Kind::Hist,
        x: centers,
        y: counts,
        z: Vec::new(),
        spec: String::new(),
        label: String::new(),
    });
    changed(i);
    Ok(vec![])
}

fn count_arg(x: f64) -> Result<usize> {
    if !x.is_finite() || !(1.0..=100_000.0).contains(&x) || x.fract() != 0.0 {
        return err(
            "Abaco:value",
            format!("the number of bins must be a whole number from 1 to 100000, got {x}"),
        );
    }
    Ok(x as usize)
}

fn bin_centers(data: &[f64], n: usize) -> Vec<f64> {
    let (lo, hi) = data
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &x| {
            (l.min(x), h.max(x))
        });
    if !lo.is_finite() {
        return (0..n).map(|k| k as f64 + 0.5).collect();
    }
    let (lo, hi) = if hi > lo {
        (lo, hi)
    } else {
        (lo - 0.5, hi + 0.5)
    };
    let w = (hi - lo) / n as f64;
    (0..n).map(|k| lo + (k as f64 + 0.5) * w).collect()
}

/// Counts values into the bins whose midpoints are `centers` (the same rule MATLAB's
/// `hist` uses: a value belongs to the nearest centre, ties go up).
fn bin_counts(data: &[f64], centers: &[f64]) -> Vec<f64> {
    let mut counts = vec![0.0; centers.len()];
    if centers.is_empty() {
        return counts;
    }
    let edges: Vec<f64> = centers.windows(2).map(|w| 0.5 * (w[0] + w[1])).collect();
    for &x in data {
        let k = edges.partition_point(|&e| e <= x);
        if let Some(c) = counts.get_mut(k) {
            *c += 1.0;
        }
    }
    counts
}

// ---------- surfaces ----------

fn grid_axis(v: &Arr<f64>, along_rows: bool, n: usize, name: &str) -> Result<Vec<f64>> {
    let xs = if v.is_vector() {
        v.d.clone()
    } else if along_rows {
        v.row_vec(0)
    } else {
        v.column(0)
    };
    if xs.len() != n {
        return err("Abaco:size", format!("`{name}`: the grid coordinates have {} values but the data has {n} in that direction", xs.len()));
    }
    Ok(xs)
}

fn surface(i: &mut Interp, a: &[Value], flat: bool, name: &str) -> Out {
    arity(a, 1, 4, name)?;
    let nums: Vec<&Value> = a.iter().filter(|v| !matches!(v, Value::Str(_))).collect();
    let (x, y, z) = match nums.as_slice() {
        [z] => {
            let z = z.to_real()?;
            (
                (1..=z.c).map(|k| k as f64).collect::<Vec<_>>(),
                (1..=z.r).map(|k| k as f64).collect::<Vec<_>>(),
                z,
            )
        }
        [x, y, z, ..] => {
            let z = z.to_real()?;
            (
                grid_axis(&x.to_real()?, true, z.c, name)?,
                grid_axis(&y.to_real()?, false, z.r, name)?,
                z,
            )
        }
        _ => return err("Abaco:arity", format!("`{name}` takes Z, or X, Y and Z")),
    };
    if z.r < 2 || z.c < 2 {
        return err(
            "Abaco:size",
            format!(
                "`{name}` needs a matrix of at least 2 by 2, got {}x{}",
                z.r, z.c
            ),
        );
    }
    let mut zr = Vec::with_capacity(z.d.len());
    for r in 0..z.r {
        zr.extend(z.row_vec(r));
    }
    let ax = target(i);
    ax.surfaces.push(Surface { x, y, z: zr, flat });
    changed(i);
    Ok(vec![])
}

fn f_surf(i: &mut Interp, a: &[Value], _: usize) -> Out {
    surface(i, a, false, "surf")
}

fn f_mesh(i: &mut Interp, a: &[Value], _: usize) -> Out {
    surface(i, a, false, "mesh")
}

fn f_contour(i: &mut Interp, a: &[Value], _: usize) -> Out {
    surface(i, a, true, "contour")
}

fn f_imagesc(i: &mut Interp, a: &[Value], _: usize) -> Out {
    surface(i, a, true, "imagesc")
}

fn f_fplot(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "fplot")?;
    let (lo, hi) = match a.get(1) {
        Some(v) => {
            let r = v.to_real()?;
            match r.d.as_slice() {
                [lo, hi] if lo.is_finite() && hi.is_finite() && lo < hi => (*lo, *hi),
                _ => return err("Abaco:value", "`fplot` needs an interval [a b] with a < b"),
            }
        }
        None => (-5.0, 5.0),
    };
    let n = 400usize;
    let xs: Vec<f64> = (0..n)
        .map(|k| lo + (hi - lo) * k as f64 / (n - 1) as f64)
        .collect();
    let ys = eval_function(i, &a[0], &xs)?;
    let ax = target(i);
    ax.series.push(Series {
        kind: Kind::Line,
        x: xs,
        y: ys,
        z: Vec::new(),
        spec: String::new(),
        label: String::new(),
    });
    changed(i);
    Ok(vec![])
}

fn eval_function(i: &mut Interp, f: &Value, xs: &[f64]) -> Result<Vec<f64>> {
    let call = |i: &mut Interp, args: Vec<Value>| -> Result<Value> {
        match f {
            Value::Str(name) => i
                .call_function(name, args, 1)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    crate::error::Error::new("Abaco:function", "the function returned nothing")
                }),
            other => i.call1(other, args),
        }
    };
    // Try the whole vector at once, which is fast for vectorised functions.
    if let Ok(v) = call(i, vec![Value::row(xs.to_vec())]) {
        if let Ok(r) = v.to_real() {
            if r.len() == xs.len() {
                return Ok(r.d);
            }
        }
    }
    let mut out = Vec::with_capacity(xs.len());
    for &x in xs {
        i.charge(1)?;
        let v = call(i, vec![Value::num(x)])?;
        out.push(v.scalar_f64("the value of the plotted function")?);
    }
    Ok(out)
}

// ---------- labels, limits and layout ----------

fn text_command(i: &mut Interp, a: &[Value], name: &str, set: fn(&mut Axes, String)) -> Out {
    arity(a, 1, usize::MAX, name)?;
    let text = match &a[0] {
        Value::Str(s) => s.clone(),
        Value::Num(_) => super::text::sprintf("%g", &a[..1])?,
        Value::Cell(c) => {
            c.d.iter()
                .filter_map(|v| {
                    if let Value::Str(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        other => {
            return err(
                "Abaco:type",
                format!("`{name}` needs text, got {}", other.describe()),
            )
        }
    };
    set(i.figures.cur().ax(), text);
    changed(i);
    Ok(vec![])
}

fn f_title(i: &mut Interp, a: &[Value], _: usize) -> Out {
    text_command(i, a, "title", |ax, s| ax.title = s)
}

fn f_xlabel(i: &mut Interp, a: &[Value], _: usize) -> Out {
    text_command(i, a, "xlabel", |ax, s| ax.xlabel = s)
}

fn f_ylabel(i: &mut Interp, a: &[Value], _: usize) -> Out {
    text_command(i, a, "ylabel", |ax, s| ax.ylabel = s)
}

fn f_zlabel(i: &mut Interp, a: &[Value], _: usize) -> Out {
    text_command(i, a, "zlabel", |ax, s| ax.zlabel = s)
}

fn f_legend(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let labels: Vec<String> = match a {
        [Value::Cell(c)] => {
            c.d.iter()
                .filter_map(|v| {
                    if let Value::Str(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => a
            .iter()
            .filter_map(|v| {
                if let Value::Str(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect(),
    };
    let ax = i.figures.cur().ax();
    if is_flag(a, 0, "off") || is_flag(a, 0, "hide") {
        ax.legend = false;
    } else {
        ax.legend = true;
        let labels: Vec<String> = labels
            .into_iter()
            .filter(|l| {
                !matches!(l.as_str(), "on" | "show" | "Location" | "location")
                    && !l.ends_with("outside")
                    && !is_location(l)
            })
            .collect();
        for (s, l) in ax.series.iter_mut().zip(labels) {
            s.label = l;
        }
    }
    changed(i);
    Ok(vec![])
}

fn is_location(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "north"
            | "south"
            | "east"
            | "west"
            | "northeast"
            | "northwest"
            | "southeast"
            | "southwest"
            | "best"
            | "bestoutside"
    )
}

fn toggle(args: &[Value], current: bool, name: &str) -> Result<bool> {
    match args.first() {
        None => Ok(!current),
        Some(Value::Str(s)) if s.eq_ignore_ascii_case("on") => Ok(true),
        Some(Value::Str(s)) if s.eq_ignore_ascii_case("off") => Ok(false),
        Some(other) => err(
            "Abaco:argument",
            format!("`{name}` takes `on` or `off`, got {}", other.describe()),
        ),
    }
}

fn f_grid(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let ax = i.figures.cur().ax();
    ax.grid = toggle(a, ax.grid, "grid")?;
    changed(i);
    Ok(vec![])
}

fn f_hold(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let fig = i.figures.cur();
    fig.hold = toggle(a, fig.hold, "hold")?;
    Ok(vec![])
}

fn limits_arg(v: &Value, name: &str) -> Result<Option<(f64, f64)>> {
    if let Value::Str(s) = v {
        if matches!(s.as_str(), "auto" | "tight") {
            return Ok(None);
        }
        return err(
            "Abaco:argument",
            format!("`{name}` takes [min max] or `auto`, got `{s}`"),
        );
    }
    let r = v.to_real()?;
    match r.d.as_slice() {
        [lo, hi] if lo < hi && !lo.is_nan() && !hi.is_nan() => Ok(Some((*lo, *hi))),
        _ => err(
            "Abaco:value",
            format!("`{name}` needs two values with min < max"),
        ),
    }
}

fn data_range(ax: &Axes, horizontal: bool) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in &ax.series {
        for &v in if horizontal { &s.x } else { &s.y } {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    if lo.is_finite() {
        (lo, hi)
    } else {
        (0.0, 1.0)
    }
}

fn lim_command(i: &mut Interp, a: &[Value], nargout: usize, horizontal: bool, name: &str) -> Out {
    arity(a, 0, 1, name)?;
    let ax = i.figures.cur().ax();
    if let Some(v) = a.first() {
        let l = limits_arg(v, name)?;
        if horizontal {
            ax.xlim = l;
        } else {
            ax.ylim = l;
        }
        changed(i);
        return Ok(vec![]);
    }
    let (lo, hi) =
        if horizontal { ax.xlim } else { ax.ylim }.unwrap_or_else(|| data_range(ax, horizontal));
    if nargout == 0 {
        return Ok(vec![]);
    }
    one(Value::row(vec![lo, hi]))
}

fn f_xlim(i: &mut Interp, a: &[Value], n: usize) -> Out {
    lim_command(i, a, n, true, "xlim")
}

fn f_ylim(i: &mut Interp, a: &[Value], n: usize) -> Out {
    lim_command(i, a, n, false, "ylim")
}

fn f_axis(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 0, 1, "axis")?;
    let ax = i.figures.cur().ax();
    match a.first() {
        None => {}
        Some(Value::Str(s)) => match s.as_str() {
            "equal" | "square" | "image" => ax.equal = true,
            "normal" => ax.equal = false,
            "tight" | "auto" => {
                ax.xlim = None;
                ax.ylim = None;
            }
            "on" | "off" | "xy" | "ij" | "manual" | "vis3d" | "fill" => {}
            other => return err("Abaco:argument", format!("`axis` does not know `{other}`; try equal, tight, auto or [xmin xmax ymin ymax]")),
        },
        Some(v) => {
            let r = v.to_real()?;
            match r.d.as_slice() {
                [x0, x1, y0, y1, ..] if x0 < x1 && y0 < y1 => {
                    ax.xlim = Some((*x0, *x1));
                    ax.ylim = Some((*y0, *y1));
                }
                _ => return err("Abaco:value", "`axis` needs [xmin xmax ymin ymax] with min < max"),
            }
        }
    }
    changed(i);
    Ok(vec![])
}

fn f_figure(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 0, 1, "figure")?;
    let n = match a.first() {
        None => i.figures.list.iter().map(|(k, _)| *k).max().unwrap_or(0) + 1,
        Some(Value::Str(s)) => s.trim().parse::<usize>().map_err(|_| {
            crate::error::Error::new(
                "Abaco:argument",
                format!("`figure` needs a figure number, got `{s}`"),
            )
        })?,
        Some(v) => v.scalar_usize("the figure number")?,
    };
    if n == 0 || n > 1000 {
        return err("Abaco:value", "figure numbers go from 1 to 1000");
    }
    i.figures.select(n);
    changed(i);
    if nargout > 0 {
        return num(n as f64);
    }
    Ok(vec![])
}

fn f_gcf(i: &mut Interp, _: &[Value], _: usize) -> Out {
    i.figures.cur();
    num(i.figures.current as f64)
}

fn f_close(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 0, 1, "close")?;
    let which = match a.first() {
        None => (i.figures.current != 0).then_some(i.figures.current),
        Some(Value::Str(s)) if s == "all" => None,
        Some(Value::Str(s)) => Some(s.trim().parse::<usize>().map_err(|_| {
            crate::error::Error::new(
                "Abaco:argument",
                format!("`close` takes a figure number or `all`, got `{s}`"),
            )
        })?),
        Some(v) => Some(v.scalar_usize("the figure number")?),
    };
    if a.is_empty() && which.is_none() {
        return Ok(vec![]);
    }
    let closing = which.unwrap_or(0);
    i.figures.close(which);
    i.emit(Event::Figure(closing));
    Ok(vec![])
}

fn f_clf(i: &mut Interp, _: &[Value], _: usize) -> Out {
    let fig = i.figures.cur();
    *fig = Figure::default();
    changed(i);
    Ok(vec![])
}

fn f_cla(i: &mut Interp, _: &[Value], _: usize) -> Out {
    *i.figures.cur().ax() = Axes::default();
    changed(i);
    Ok(vec![])
}

fn f_subplot(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "subplot")?;
    let (r, c, k) = match a {
        [v] => {
            let n = v.scalar_usize("the subplot code")?;
            if !(111..=999).contains(&n) {
                return err(
                    "Abaco:value",
                    "`subplot(rck)` takes a three-digit code such as 212",
                );
            }
            (n / 100, (n / 10) % 10, n % 10)
        }
        [r, c, k] => (
            r.scalar_usize("the rows")?,
            c.scalar_usize("the columns")?,
            k.scalar_usize("the cell")?,
        ),
        _ => {
            return err(
                "Abaco:arity",
                "`subplot` takes (rows, cols, cell) or a three-digit code",
            )
        }
    };
    if r == 0 || c == 0 || r * c > MAX_CELLS {
        return err(
            "Abaco:value",
            format!("`subplot` grids hold from 1 to {MAX_CELLS} cells, got {r}x{c}"),
        );
    }
    if k == 0 || k > r * c {
        return err(
            "Abaco:index",
            format!("`subplot` cell {k} is outside a {r}x{c} grid"),
        );
    }
    let fig = i.figures.cur();
    if fig.rows != r || fig.cols != c {
        fig.rows = r;
        fig.cols = c;
        fig.axes.resize(r * c, Axes::default());
    }
    fig.current = k - 1;
    changed(i);
    Ok(vec![])
}

/// Commands that are accepted so that scripts written for other environments run.
fn f_ignored(_: &mut Interp, _: &[Value], _: usize) -> Out {
    Ok(vec![])
}

fn f_saveas(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "saveas")?;
    let _ = str_arg(a, a.len().saturating_sub(1).min(1), "saveas")?;
    i.warn("`saveas` does not write files from a script; use Export in the plot window instead");
    Ok(vec![])
}

pub const TABLE: &[(&str, Bf)] = &[
    ("axis", f_axis),
    ("bar", f_bar),
    ("box", f_ignored),
    ("cla", f_cla),
    ("clf", f_clf),
    ("close", f_close),
    ("colorbar", f_ignored),
    ("colormap", f_ignored),
    ("contour", f_contour),
    ("figure", f_figure),
    ("fplot", f_fplot),
    ("gcf", f_gcf),
    ("grid", f_grid),
    ("hist", f_hist),
    ("histogram", f_hist),
    ("hold", f_hold),
    ("imagesc", f_imagesc),
    ("legend", f_legend),
    ("loglog", f_loglog),
    ("mesh", f_mesh),
    ("pcolor", f_contour),
    ("plot", f_plot),
    ("plot3", f_plot3),
    ("saveas", f_saveas),
    ("scatter", f_scatter),
    ("semilogx", f_semilogx),
    ("semilogy", f_semilogy),
    ("shading", f_ignored),
    ("stairs", f_stairs),
    ("stem", f_stem),
    ("subplot", f_subplot),
    ("surf", f_surf),
    ("title", f_title),
    ("view", f_ignored),
    ("xlabel", f_xlabel),
    ("xlim", f_xlim),
    ("ylabel", f_ylabel),
    ("ylim", f_ylim),
    ("zlabel", f_zlabel),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_counts_follow_the_nearest_centre() {
        let data = [1.0, 2.0, 2.0, 3.0, 9.0];
        let centers = bin_centers(&data, 4);
        let counts = bin_counts(&data, &centers);
        assert_eq!(counts.iter().sum::<f64>(), 5.0);
        assert_eq!(counts.len(), 4);
        assert_eq!(counts[3], 1.0);
    }

    #[test]
    fn matrix_y_plots_one_series_per_column() {
        let y = Arr::new(3, 2, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let s = split_series(None, &y, "plot").unwrap_or_default();
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].1, vec![4.0, 5.0, 6.0]);
        assert_eq!(s[0].0, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn mismatched_lengths_are_an_error() {
        let x = Arr::row(vec![1.0, 2.0]);
        let y = Arr::row(vec![1.0, 2.0, 3.0]);
        assert!(split_series(Some(&x), &y, "plot").is_err());
    }
}
