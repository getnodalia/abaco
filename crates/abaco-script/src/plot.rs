// SPDX-License-Identifier: MIT OR Apache-2.0
//! The data behind plotting commands. The interpreter only records what was asked for;
//! the host (the app, or the CLI exporting an image) draws it.

/// What kind of mark a series draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Connected line segments.
    Line,
    /// Markers only.
    Scatter,
    /// Vertical stems.
    Stem,
    /// Staircase.
    Stairs,
    /// Bars from zero.
    Bar,
    /// A histogram (bars already binned by the command).
    Hist,
    /// A 3-D curve.
    Line3,
}

/// One drawn series.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// How it is drawn.
    pub kind: Kind,
    /// Horizontal values.
    pub x: Vec<f64>,
    /// Vertical values.
    pub y: Vec<f64>,
    /// Depth values for 3-D series (empty otherwise).
    pub z: Vec<f64>,
    /// The `plot` line-spec string, for example `r--o`.
    pub spec: String,
    /// The legend label, when given.
    pub label: String,
}

/// A surface or image on a regular grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    /// Grid x values per column.
    pub x: Vec<f64>,
    /// Grid y values per row.
    pub y: Vec<f64>,
    /// Heights, row-major with `y.len()` rows and `x.len()` columns.
    pub z: Vec<f64>,
    /// `true` for a contour/heat-map style, `false` for a 3-D mesh.
    pub flat: bool,
}

/// One set of axes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Axes {
    /// Series in drawing order.
    pub series: Vec<Series>,
    /// Surfaces in drawing order.
    pub surfaces: Vec<Surface>,
    /// Title.
    pub title: String,
    /// Horizontal axis label.
    pub xlabel: String,
    /// Vertical axis label.
    pub ylabel: String,
    /// Depth axis label.
    pub zlabel: String,
    /// Grid lines on.
    pub grid: bool,
    /// Legend on.
    pub legend: bool,
    /// Horizontal limits, when set.
    pub xlim: Option<(f64, f64)>,
    /// Vertical limits, when set.
    pub ylim: Option<(f64, f64)>,
    /// Logarithmic horizontal axis.
    pub xlog: bool,
    /// Logarithmic vertical axis.
    pub ylog: bool,
    /// Equal scaling on both axes.
    pub equal: bool,
}

/// A figure: a grid of axes.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    /// Rows of the subplot grid.
    pub rows: usize,
    /// Columns of the subplot grid.
    pub cols: usize,
    /// The axes in row-major order; grows as `subplot` selects cells.
    pub axes: Vec<Axes>,
    /// The cell commands draw into.
    pub current: usize,
    /// `hold on`.
    pub hold: bool,
}

impl Default for Figure {
    fn default() -> Figure {
        Figure {
            rows: 1,
            cols: 1,
            axes: vec![Axes::default()],
            current: 0,
            hold: false,
        }
    }
}

impl Figure {
    /// The axes commands currently draw into.
    pub fn ax(&mut self) -> &mut Axes {
        if self.current >= self.axes.len() {
            self.axes.resize(self.current + 1, Axes::default());
        }
        let i = self.current;
        &mut self.axes[i]
    }
}

/// All figures of an interpreter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Figures {
    /// Open figures as `(number, figure)`.
    pub list: Vec<(usize, Figure)>,
    /// The current figure number (0 when none).
    pub current: usize,
}

impl Figures {
    /// The current figure, creating figure 1 when none exists.
    pub fn cur(&mut self) -> &mut Figure {
        if self.current == 0 {
            self.select(1);
        }
        let n = self.current;
        self.figure(n)
    }

    /// Makes figure `n` current, creating it when new.
    pub fn select(&mut self, n: usize) {
        if !self.list.iter().any(|(k, _)| *k == n) {
            self.list.push((n, Figure::default()));
        }
        self.current = n;
    }

    /// The figure numbered `n`, created when new.
    pub fn figure(&mut self, n: usize) -> &mut Figure {
        if !self.list.iter().any(|(k, _)| *k == n) {
            self.list.push((n, Figure::default()));
        }
        let i = self.list.iter().position(|(k, _)| *k == n).unwrap_or(0);
        &mut self.list[i].1
    }

    /// Closes one figure, or all of them.
    pub fn close(&mut self, n: Option<usize>) {
        match n {
            Some(n) => {
                self.list.retain(|(k, _)| *k != n);
                if self.current == n {
                    self.current = self.list.last().map_or(0, |(k, _)| *k);
                }
            }
            None => {
                self.list.clear();
                self.current = 0;
            }
        }
    }
}
