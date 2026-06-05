use clap::{Args, Parser, Subcommand};
use memspark_core::config::{init_config, load_config_or_default};
use memspark_core::{
    default_config_path, enumerate_processes, format_report_text, format_report_text_with_options,
    get_memory_snapshot, is_running_as_admin, load_last_report, optimize, run_elevated_and_wait,
    save_developer_log, save_report, AppConfig, MemSparkError, OptimizeReport,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(name = "memspark")]
#[command(version, about = "Explainable Windows working-set trimming tool")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Status,
    Trim(TrimArgs),
    Config(ConfigArgs),
    Report(ReportArgs),
}

#[derive(Args)]
struct TrimArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long, help = "Show all skipped processes in dry-run output")]
    verbose: bool,
    #[arg(long, hide = true)]
    elevated_child: bool,
    #[arg(long, hide = true)]
    elevated_output: Option<PathBuf>,
}

#[derive(Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand)]
enum ConfigCommand {
    Init {
        #[arg(long)]
        force: bool,
    },
    Show,
}

#[derive(Args)]
struct ReportArgs {
    #[command(subcommand)]
    command: ReportCommand,
}

#[derive(Subcommand)]
enum ReportCommand {
    Last,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), MemSparkError> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status => status(),
        Commands::Trim(args) => trim(cli.config, args),
        Commands::Config(args) => config_command(cli.config, args),
        Commands::Report(args) => report_command(cli.config, args),
    }
}

fn status() -> Result<(), MemSparkError> {
    let snapshot = get_memory_snapshot()?;
    let processes = enumerate_processes()?;
    println!("MemSpark Status");
    println!(
        "Physical Used: {} / {} ({:.1}%)",
        format_gb(snapshot.used_physical()),
        format_gb(snapshot.total_physical),
        snapshot.used_physical() as f64 * 100.0 / snapshot.total_physical.max(1) as f64
    );
    println!("Available:     {}", format_gb(snapshot.available_physical));
    println!("System Cache:  {}", format_gb(snapshot.system_cache));
    println!(
        "Commit:        {} / {}",
        format_gb(snapshot.commit_total),
        format_gb(snapshot.commit_limit)
    );
    println!(
        "Processes:     {} (enumerated: {})",
        snapshot.process_count,
        processes.len()
    );
    Ok(())
}

fn trim(config_path: Option<PathBuf>, args: TrimArgs) -> Result<(), MemSparkError> {
    let config = load_config_or_default(config_path.as_deref())?;
    if !args.dry_run && !args.elevated_child && !is_running_as_admin() {
        return run_elevated_trim(config_path, args);
    }
    if !args.dry_run {
        eprintln!(
            "warning: memory optimization targets 25% memory load and may terminate non-system background processes when trimming and system release do not reach the target."
        );
    }
    let report = optimize(args.dry_run, &config)?;
    if let Some(path) = &args.elevated_output {
        write_json(path, &report)?;
        return Ok(());
    }
    println!("{}", format_report_text_with_options(&report, args.verbose));
    save_report_artifacts(&config, &report);
    Ok(())
}

fn run_elevated_trim(config_path: Option<PathBuf>, args: TrimArgs) -> Result<(), MemSparkError> {
    let output = elevated_output_path("trim");
    let exe = std::env::current_exe()?;
    let mut params = Vec::new();
    if let Some(path) = &config_path {
        params.push("--config".to_owned());
        params.push(path.display().to_string());
    }
    params.push("trim".to_owned());
    if args.verbose {
        params.push("--verbose".to_owned());
    }
    params.push("--elevated-child".to_owned());
    params.push("--elevated-output".to_owned());
    params.push(output.display().to_string());

    eprintln!("requesting administrator permission for memory optimization...");
    let exit_code = run_elevated_and_wait(&exe.display().to_string(), &join_windows_args(&params))
        .map_err(|err| MemSparkError::WinApi(err.to_string()))?;
    if exit_code != 0 {
        return Err(MemSparkError::WinApi(format!(
            "elevated memory optimization exited with code {exit_code}"
        )));
    }
    let content = fs::read_to_string(&output)?;
    let report: OptimizeReport = serde_json::from_str(&content)?;
    println!("{}", format_report_text_with_options(&report, args.verbose));
    let config = load_config_or_default(config_path.as_deref())?;
    save_report_artifacts(&config, &report);
    let _ = fs::remove_file(output);
    Ok(())
}

fn save_report_artifacts(config: &AppConfig, report: &OptimizeReport) {
    match save_report(config, report) {
        Ok(Some(path)) => println!("Report saved: {}", path.display()),
        Ok(None) => {}
        Err(err) => eprintln!("warning: report was generated but could not be saved: {err}"),
    }
    match save_developer_log(config, report) {
        Ok(Some(path)) => println!("Developer log saved: {}", path.display()),
        Ok(None) => {}
        Err(err) => eprintln!("warning: developer log could not be saved: {err}"),
    }
}

fn config_command(config_path: Option<PathBuf>, args: ConfigArgs) -> Result<(), MemSparkError> {
    match args.command {
        ConfigCommand::Init { force } => {
            let target = config_path.clone().unwrap_or_else(default_config_path);
            let existed = target.exists();
            let path = init_config(config_path.as_deref(), force)?;
            println!("Config ready: {}", path.display());
            if !force && existed {
                println!("Existing config was kept. Use --force to overwrite with defaults.");
            }
        }
        ConfigCommand::Show => {
            let config = load_config_or_default(config_path.as_deref())?;
            println!("{}", toml::to_string_pretty(&config)?);
            if config_path.is_none() {
                println!("# Default path: {}", default_config_path().display());
            }
        }
    }
    Ok(())
}

fn report_command(config_path: Option<PathBuf>, args: ReportArgs) -> Result<(), MemSparkError> {
    let config = load_config_or_default(config_path.as_deref())?;
    match args.command {
        ReportCommand::Last => {
            let report = load_last_report(&config)?;
            println!("{}", format_report_text(&report));
        }
    }
    Ok(())
}

fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

fn write_json<T: serde::Serialize>(path: &PathBuf, value: &T) -> Result<(), MemSparkError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn elevated_output_path(kind: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!("memspark-{kind}-{}-{now}.json", std::process::id()))
}

fn join_windows_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_owned();
    }
    if !arg.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return arg.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}
