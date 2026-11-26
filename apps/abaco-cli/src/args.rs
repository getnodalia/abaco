// SPDX-License-Identifier: MIT OR Apache-2.0
//! The command line.

/// What the command line asked for.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// Print help (for a command, or in general).
    Help(Option<&'static str>),
    /// Print the version.
    Version,
    /// Run a command.
    Command(Command),
}

/// Options shared by the commands.
#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub struct Options {
    /// `--json`: machine-readable output.
    pub json: bool,
    /// `--digits N`: significant digits, 1 to 17.
    pub digits: Option<usize>,
}

/// A command with its arguments.
#[derive(Debug, PartialEq)]
pub enum Command {
    /// `abaco eval <expr>`.
    Eval(String, Options),
    /// `abaco run <file>`.
    Run(String, Options),
    /// `abaco script <file>` or `abaco script --code <source>`.
    Script(ScriptSource, Options),
    /// `abaco sim <model.json>`.
    Sim(SimArgs, Options),
    /// `abaco repl`.
    Repl,
    /// `abaco functions`.
    Functions(Options),
    /// `abaco units list`.
    UnitsList(Options),
    /// `abaco units convert <quantity> <unit>`.
    UnitsConvert(String, String, Options),
}

/// What `abaco sim` was asked to run.
#[derive(Debug, PartialEq, Clone)]
pub struct SimArgs {
    /// The block-diagram file (JSON).
    pub file: String,
    /// `--stop T`: overrides the model's stop time.
    pub stop: Option<f64>,
    /// `--csv FILE`: also write every recorded signal there.
    pub csv: Option<String>,
}

/// Where a script's source comes from.
#[derive(Debug, PartialEq, Clone)]
pub enum ScriptSource {
    /// A file; `-` reads standard input.
    File(String),
    /// Source text given on the command line.
    Code(String),
}

const COMMANDS: [&str; 8] = [
    "eval",
    "run",
    "script",
    "sim",
    "repl",
    "functions",
    "units",
    "help",
];

/// Parses the arguments after the program name.
pub fn parse(argv: &[String]) -> Result<Parsed, String> {
    let mut opts = Options::default();
    let mut positional: Vec<String> = Vec::new();
    let mut help = false;
    let mut version = false;
    let mut only_positional = false;
    let mut code: Option<String> = None;
    let mut stop: Option<f64> = None;
    let mut csv: Option<String> = None;
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if only_positional {
            positional.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--" => only_positional = true,
            "--json" => opts.json = true,
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            "--code" => {
                code = Some(
                    it.next()
                        .ok_or_else(|| "`--code` needs the script text, in quotes".to_string())?
                        .clone(),
                );
            }
            "--stop" => {
                let v = it.next().ok_or_else(|| {
                    "`--stop` needs the stop time, for example: --stop 10".to_string()
                })?;
                stop = Some(match v.parse::<f64>() {
                    Ok(t) if t.is_finite() => t,
                    _ => return Err(format!("`--stop` needs a finite number, got `{v}`")),
                });
            }
            "--csv" => {
                csv = Some(
                    it.next()
                        .ok_or_else(|| "`--csv` needs a file name".to_string())?
                        .clone(),
                );
            }
            "--digits" => {
                let v = it
                    .next()
                    .ok_or_else(|| "`--digits` needs a number from 1 to 17".to_string())?;
                opts.digits = Some(digits(v)?);
            }
            s if s.starts_with("--digits=") => {
                opts.digits = Some(digits(s.trim_start_matches("--digits="))?);
            }
            s if s.starts_with("--") => {
                return Err(format!("unknown option `{s}`"));
            }
            // A lone `-x` is an expression such as `-3 + 2`, not an option.
            _ => positional.push(a.clone()),
        }
    }
    let command = positional.first().map(String::as_str);
    if version && command.is_none() {
        return Ok(Parsed::Version);
    }
    if help {
        return Ok(Parsed::Help(command.and_then(topic)));
    }
    let mut rest = positional.iter().skip(1);
    match command {
        None => Ok(Parsed::Help(None)),
        Some("help") => Ok(Parsed::Help(rest.next().and_then(|t| topic(t)))),
        Some("eval") => {
            let expr = rest.next().ok_or(
                "`eval` needs an expression, for example: abaco eval '10 kg * 9.81 m/s^2 -> N'",
            )?;
            if rest.next().is_some() {
                return Err(
                    "`eval` takes one expression; put it in quotes so the shell keeps it whole"
                        .into(),
                );
            }
            Ok(Parsed::Command(Command::Eval(expr.clone(), opts)))
        }
        Some("run") => {
            let file = rest
                .next()
                .ok_or("`run` needs a file, for example: abaco run beam.abaco")?;
            if rest.next().is_some() {
                return Err("`run` takes one file".into());
            }
            Ok(Parsed::Command(Command::Run(file.clone(), opts)))
        }
        Some("script") => match (code, rest.next()) {
            (Some(src), None) => Ok(Parsed::Command(Command::Script(
                ScriptSource::Code(src),
                opts,
            ))),
            (Some(_), Some(_)) => Err("`script` takes a file or --code, not both".into()),
            (None, Some(file)) => {
                if rest.next().is_some() {
                    return Err("`script` takes one file".into());
                }
                Ok(Parsed::Command(Command::Script(
                    ScriptSource::File(file.clone()),
                    opts,
                )))
            }
            (None, None) => Err(
                "`script` needs a file (or `-` for standard input) or --code '<source>', for example: abaco script analysis.m"
                    .into(),
            ),
        },
        Some("sim") => {
            let file = rest
                .next()
                .ok_or("`sim` needs a model file, for example: abaco sim servo.json")?;
            if rest.next().is_some() {
                return Err("`sim` takes one model file".into());
            }
            Ok(Parsed::Command(Command::Sim(
                SimArgs {
                    file: file.clone(),
                    stop,
                    csv,
                },
                opts,
            )))
        }
        Some("repl") => {
            if rest.next().is_some() {
                return Err("`repl` takes no arguments".into());
            }
            Ok(Parsed::Command(Command::Repl))
        }
        Some("functions") => {
            if rest.next().is_some() {
                return Err("`functions` takes no arguments".into());
            }
            Ok(Parsed::Command(Command::Functions(opts)))
        }
        Some("units") => match rest.next().map(String::as_str) {
            Some("list") => {
                if rest.next().is_some() {
                    return Err("`units list` takes no arguments".into());
                }
                Ok(Parsed::Command(Command::UnitsList(opts)))
            }
            Some("convert") => {
                let q = rest.next().ok_or(
                    "`units convert` needs a quantity and a unit, for example: abaco units convert '60 mph' 'm/s'",
                )?;
                let u = rest
                    .next()
                    .ok_or("`units convert` needs the unit to convert to")?;
                if rest.next().is_some() {
                    return Err(
                        "`units convert` takes a quantity and a unit, both in quotes".into(),
                    );
                }
                Ok(Parsed::Command(Command::UnitsConvert(
                    q.clone(),
                    u.clone(),
                    opts,
                )))
            }
            Some(other) => Err(format!(
                "unknown `units` command `{other}`; use `list` or `convert`"
            )),
            None => Err("`units` needs `list` or `convert`".into()),
        },
        Some(other) => {
            let hint = COMMANDS
                .iter()
                .find(|c| c.starts_with(&other.chars().take(2).collect::<String>()))
                .map(|c| format!("; did you mean `{c}`?"))
                .unwrap_or_default();
            Err(format!("unknown command `{other}`{hint}"))
        }
    }
}

fn digits(v: &str) -> Result<usize, String> {
    match v.parse::<usize>() {
        Ok(n) if (1..=17).contains(&n) => Ok(n),
        _ => Err(format!(
            "`--digits` needs a whole number from 1 to 17, got `{v}`"
        )),
    }
}

fn topic(name: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|c| **c == name).copied()
}

/// The help text, general or for one command.
pub fn help(topic: Option<&'static str>) -> String {
    let general = "\
abaco: evaluate maths with units from the command line

USAGE
    abaco eval '<expression>' [--json] [--digits N]
    abaco run <file.abaco> [--json] [--digits N]
    abaco script <file.m | -> | --code '<source>'
    abaco sim <model.json> [--stop T] [--csv out.csv] [--json] [--digits N]
    abaco repl
    abaco functions [--json]
    abaco units list [--json]
    abaco units convert '<quantity>' '<unit>' [--json] [--digits N]

OPTIONS
    --json        machine-readable output
    --digits N    significant digits to show, 1 to 17 (default 6, or the document's setting)
    -h, --help    show help
    -V, --version show the version

EXIT CODES
    0 success; 1 an expression, cell or conversion failed; 2 the command line or file was unusable

Run `abaco help <command>` for details.";
    match topic {
        Some("eval") => "\
abaco eval '<expression>' [--json] [--digits N]

Evaluates one expression (statements separated by `;` or newlines are allowed; the value of the
last one is shown). Put the expression in quotes. An expression that starts with `-` works as is.

    abaco eval '10 kg * 9.81 m/s^2 -> N'      98.1 N
    abaco eval 'sin(30 deg)'                  0.5
    abaco eval 'det([1,2;3,4])'               -2

On an error the message, line and column go to standard error and the exit code is 1.
With --json the result or the error is a JSON object on standard output."
            .to_string(),
        Some("run") => "\
abaco run <file.abaco> [--json] [--digits N]

Evaluates every cell of a document in dependency order and prints each result. A cell that
fails prints its error with the line and column inside the cell. The exit code is 1 when any
cell has an error. With --json the whole report is printed as JSON."
            .to_string(),
        Some("script") => "\
abaco script <file | -> 
abaco script --code '<source>'

Runs a script in Ábaco Script, the matrix language: variables, matrices, functions, loops,
cells, structs and the numerical library (`abaco functions` lists it). Printed output goes to
standard output, warnings to standard error. `-` reads the script from standard input.

    abaco script --code 'A = [4 -2; 1 1]; disp(eig(A))'

On an error the message, line and column go to standard error and the exit code is 1."
            .to_string(),
        Some("sim") => "\
abaco sim <model.json> [--stop T] [--csv out.csv] [--json] [--digits N]

Simulates a block diagram (Ábaco Blocks, JSON) with the model's own solver and prints the final
value of every logged signal (scopes, outports, ToWorkspace) with the solver's statistics.
--stop T overrides the stop time; --csv writes time and every signal to a file; --json prints
the whole run (times and samples). The exit code is 1 when the model cannot be compiled or the
run fails (algebraic loop, a non-finite signal, a step that cannot be taken), with the block named.

    abaco sim servo.json --stop 5 --csv servo.csv"
            .to_string(),
        Some("repl") => "\
abaco repl

Reads Ábaco Script statements from standard input and runs them in one workspace, showing the
value of each statement that has no `;`. A block (`for`, `if`, `function`) is run once its `end`
has been typed. Exit with Ctrl-D."
            .to_string(),
        Some("functions") => "\
abaco functions [--json]

Lists every built-in function of Ábaco Script, one per line (or as a JSON array)."
            .to_string(),
        Some("units") => "\
abaco units list [--json]
abaco units convert '<quantity>' '<unit>' [--json] [--digits N]

`list` shows every unit symbol the engine knows, with its name and dimension. `convert`
converts a quantity: abaco units convert '60 mph' 'm/s'"
            .to_string(),
        _ => general.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Parsed, String> {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_sim_with_its_options() {
        assert_eq!(
            p(&["sim", "m.json", "--stop", "2.5", "--csv", "o.csv", "--json"]),
            Ok(Parsed::Command(Command::Sim(
                SimArgs {
                    file: "m.json".into(),
                    stop: Some(2.5),
                    csv: Some("o.csv".into())
                },
                Options {
                    json: true,
                    digits: None
                }
            )))
        );
        assert!(p(&["sim"]).is_err());
        assert!(p(&["sim", "a.json", "b.json"]).is_err());
        assert!(p(&["sim", "a.json", "--stop", "soon"]).is_err());
        assert!(p(&["sim", "a.json", "--stop", "inf"]).is_err());
        assert!(p(&["sim", "a.json", "--csv"]).is_err());
    }

    #[test]
    fn parses_commands_and_options_anywhere() {
        let o = Options {
            json: true,
            digits: Some(8),
        };
        assert_eq!(
            p(&["eval", "1+1", "--json", "--digits", "8"]),
            Ok(Parsed::Command(Command::Eval("1+1".into(), o)))
        );
        assert_eq!(
            p(&["--digits=8", "--json", "eval", "1+1"]),
            Ok(Parsed::Command(Command::Eval("1+1".into(), o)))
        );
        assert_eq!(
            p(&["eval", "-3 + 2"]),
            Ok(Parsed::Command(Command::Eval(
                "-3 + 2".into(),
                Options::default()
            )))
        );
        assert_eq!(
            p(&["eval", "--", "--x"]),
            Ok(Parsed::Command(Command::Eval(
                "--x".into(),
                Options::default()
            )))
        );
    }

    #[test]
    fn rejects_bad_usage() {
        assert!(p(&["eval"]).is_err());
        assert!(p(&["eval", "a", "b"]).is_err());
        assert!(p(&["run"]).is_err());
        assert!(p(&["units"]).is_err());
        assert!(p(&["script"]).is_err());
        assert!(p(&["script", "a.m", "b.m"]).is_err());
        assert!(p(&["script", "a.m", "--code", "x"]).is_err());
        assert!(p(&["repl", "x"]).is_err());
        assert!(p(&["units", "convert", "1 m"]).is_err());
        assert!(p(&["frobnicate"]).is_err());
        assert!(p(&["eval", "1", "--digits"]).is_err());
        assert!(p(&["eval", "1", "--digits", "0"]).is_err());
        assert!(p(&["eval", "1", "--digits", "x"]).is_err());
        assert!(p(&["eval", "1", "--digits=99"]).is_err());
        assert!(p(&["eval", "1", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_script_commands() {
        assert_eq!(
            p(&["script", "a.m"]),
            Ok(Parsed::Command(Command::Script(
                ScriptSource::File("a.m".into()),
                Options::default()
            )))
        );
        assert_eq!(
            p(&["script", "--code", "x = 1"]),
            Ok(Parsed::Command(Command::Script(
                ScriptSource::Code("x = 1".into()),
                Options::default()
            )))
        );
        assert_eq!(p(&["repl"]), Ok(Parsed::Command(Command::Repl)));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(p(&[]), Ok(Parsed::Help(None)));
        assert_eq!(p(&["--version"]), Ok(Parsed::Version));
        assert_eq!(p(&["eval", "--help"]), Ok(Parsed::Help(Some("eval"))));
        assert_eq!(p(&["help", "units"]), Ok(Parsed::Help(Some("units"))));
    }
}
