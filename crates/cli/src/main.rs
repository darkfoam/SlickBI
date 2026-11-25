//! The `slickbi-cli` entry point: argument parsing and dispatch.

use std::path::PathBuf;
use std::process::ExitCode;

use slickbi_cli::{
    OutputFormat, compile, inspect, query, render_preview, render_print, render_report_preview,
    render_svg, schema, validate,
};
use slickbi_entitlement::Entitlement;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(output) => {
            if !output.is_empty() {
                print!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, slickbi_cli::CliError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(slickbi_cli::CliError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            usage(),
        )));
    };

    match command {
        "validate" => {
            let path = path_arg(args)?;
            validate(&path)?;
            Ok(format!("{} is valid\n", path.display()))
        }
        "compile" => {
            let path = path_arg(args)?;
            let worksheet = flag(args, "--worksheet");
            compile(&path, worksheet)
        }
        "inspect" => {
            let path = path_arg(args)?;
            inspect(&path)
        }
        "schema" => Ok(format!("{}\n", schema()?)),
        "query" => {
            let path = path_arg(args)?;
            let Some(worksheet) = flag(args, "--worksheet") else {
                return Err(usage_error("query needs --worksheet <id|name>"));
            };
            let format = if flag(args, "--format") == Some("csv") {
                OutputFormat::Csv
            } else {
                OutputFormat::Table
            };
            query(&path, worksheet, format)
        }
        "render" => {
            let path = path_arg(args)?;
            let Some(worksheet) = flag(args, "--worksheet") else {
                return Err(usage_error("render needs --worksheet <id|name>"));
            };
            let out = flag(args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("chart.png"));
            let width = flag(args, "--width")
                .map(|v| v.parse().unwrap_or(800.0))
                .unwrap_or(800.0);
            let height = flag(args, "--height")
                .map(|v| v.parse().unwrap_or(600.0))
                .unwrap_or(600.0);
            if args.iter().any(|arg| arg == "--svg") {
                // Free until S03 verifies a signed entitlement token.
                render_svg(&path, worksheet, &out, (width, height), Entitlement::free())?;
            } else if args.iter().any(|arg| arg == "--print") {
                let scale = flag(args, "--scale")
                    .map(|v| v.parse().unwrap_or(2.0))
                    .unwrap_or(2.0);
                // Free until S03 verifies a signed entitlement token.
                render_print(
                    &path,
                    worksheet,
                    &out,
                    (width, height),
                    scale,
                    Entitlement::free(),
                )?;
            } else {
                render_preview(&path, worksheet, &out, (width, height))?;
            }
            Ok(format!("wrote {}\n", out.display()))
        }
        "render-report" => {
            let path = path_arg(args)?;
            let Some(report) = flag(args, "--report") else {
                return Err(usage_error("render-report needs --report <id|name>"));
            };
            let out = flag(args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("report.png"));
            let scale = flag(args, "--scale")
                .map(|v| v.parse().unwrap_or(1.0))
                .unwrap_or(1.0);
            render_report_preview(&path, report, &out, scale)?;
            Ok(format!("wrote {}\n", out.display()))
        }
        _ => Err(usage_error(&format!("unknown command '{command}'"))),
    }
}

fn path_arg(args: &[String]) -> Result<PathBuf, slickbi_cli::CliError> {
    args.iter()
        .find(|arg| !arg.starts_with("--"))
        .map(PathBuf::from)
        .ok_or_else(|| usage_error("a .scz path is required"))
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let mut values = args.iter();
    while let Some(arg) = values.next() {
        if arg == name {
            return values.next().map(String::as_str);
        }
    }
    None
}

fn usage_error(message: &str) -> slickbi_cli::CliError {
    slickbi_cli::CliError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("{message}\n\n{}", usage()),
    ))
}

fn usage() -> String {
    "\
slickbi-cli — inspect and query a SlickBI workbook

Usage:
  slickbi-cli validate FILE.scz
  slickbi-cli compile  FILE.scz [--worksheet <id|name>]
  slickbi-cli inspect  FILE.scz
  slickbi-cli schema
  slickbi-cli query    FILE.scz --worksheet <id|name> [--format table|csv]
  slickbi-cli render   FILE.scz --worksheet <id|name> --out chart.png [--width 800 --height 600]
                       [--print (Pro) --scale 2] [--svg (Pro)]
  slickbi-cli render-report FILE.scz --report <id|name> --out report.png [--scale 1]
"
    .to_owned()
}
