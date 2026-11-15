use std::path::{Path, PathBuf};
use crate::codegen::backend::CodegenBackend;
use crate::hardware;
use crate::interpret::Interpreter;
use crate::module::ModuleLoader;
use crate::semantic::env::Env;
use crate::semantic::typeck::TypeChecker;
use crate::syntax::ast::Module;

mod native;
mod upgrade;
mod watch;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Engine {
    Interp,
    Native,
}

pub enum Command {
    Run(String, bool, Engine),
    Build(String, bool),
    BuildWithInfo(String, bool),
    Init(Option<String>),
    Add(String),
    Remove(String),
    Sync,
    Info,
    Version,
    Upgrade(bool),
}

pub fn parse_args() -> Command {
    let args: Vec<String> = std::env::args().collect();
    parse_from(&args)
}

pub fn parse_from(args: &[String]) -> Command {
    if args.len() < 2 {
        eprintln!("Usage:");
        eprintln!("  yidi <file>              Run a .yk file (native build, cached)");
        eprintln!("  yidi run <file>          Run a .yk file (native build, cached)");
        eprintln!("  yidi run <file> --interp Run with the interpreter");
        eprintln!("  yidi <file> --watch      Run file and watch for changes");
        eprintln!("  yidi build <file>        Build a .yk file");
        eprintln!("  yidi build <file> --watch Rebuild on file change");
        eprintln!("  yidi info                Show hardware info");
        eprintln!("  yidi init [name]         Create a project (here, or in a new directory)");
        eprintln!("  yidi add <package>       Add a dependency (github:user/repo or a git URL)");
        eprintln!("  yidi remove <package>    Remove a dependency");
        eprintln!("  yidi sync                Install or update all dependencies");
        eprintln!("  yidi upgrade             Install the latest release");
        eprintln!("  yidi upgrade --check     Only report whether a newer release exists");
        eprintln!("  yidi --version           Show version");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "--version" | "-V" | "version" => Command::Version,
        "info" => Command::Info,
        "build" => {
            if args.len() < 3 { eprintln!("Usage: yidi build <file>"); std::process::exit(1); }
            let watch = args.iter().any(|a| a == "--watch");
            let verbose = args.iter().any(|a| a == "--info");
            if verbose { Command::BuildWithInfo(args[2].clone(), watch) }
            else { Command::Build(args[2].clone(), watch) }
        }
        "run" => {
            if args.len() < 3 { eprintln!("Usage: yidi run <file>"); std::process::exit(1); }
            Command::Run(args[2].clone(), has_flag(args, "--watch"), engine_from(args))
        }
        "add" => {
            if args.len() < 3 { eprintln!("Usage: yidi add <package>"); std::process::exit(1); }
            Command::Add(args[2].clone())
        }
        "init" => Command::Init(args.get(2).cloned()),
        "remove" => {
            if args.len() < 3 { eprintln!("Usage: yidi remove <package>"); std::process::exit(1); }
            Command::Remove(args[2].clone())
        }
        "sync" => Command::Sync,
        "upgrade" => Command::Upgrade(has_flag(args, "--check")),
        _ => {
            // backward compatibility: treat bare path as "run"
            Command::Run(args[1].clone(), has_flag(args, "--watch"), engine_from(args))
        }
    }
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn engine_from(args: &[String]) -> Engine {
    if has_flag(args, "--interp") { Engine::Interp } else { Engine::Native }
}

pub fn execute(cmd: Command) -> Result<String, String> {
    match cmd {
        Command::Run(f, watch, engine) => run_program(&PathBuf::from(f), watch, engine),
        Command::Build(f, watch) => build_program(&PathBuf::from(f), watch, &crate::codegen::backend::LlvmBackend, false),
        Command::BuildWithInfo(f, watch) => build_program(&PathBuf::from(f), watch, &crate::codegen::backend::LlvmBackend, true),
        Command::Info => show_info(),
        Command::Init(name) => crate::package::init(name.as_deref()),
        Command::Add(pkg) => crate::package::add(&pkg),
        Command::Remove(pkg) => crate::package::remove(&pkg),
        Command::Sync => crate::package::sync(),
        Command::Version => Ok(version_string()),
        Command::Upgrade(check_only) => upgrade::upgrade(check_only),
    }
}

fn version_string() -> String {
    format!("yidi {}", env!("CARGO_PKG_VERSION"))
}

fn show_info() -> Result<String, String> {
    let hw = hardware::detect();
    Ok(hw.to_string())
}

struct LoadedFile {
    path: PathBuf,
    source: String,
    module: Module,
}

fn load(path: &PathBuf) -> Result<Vec<LoadedFile>, String> {
    let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let file = path.file_name().unwrap().to_string_lossy().to_string();

    let mut loader = ModuleLoader::new(root);
    loader.load(&PathBuf::from(&file))
        .map_err(|e| e.to_string())?;

    let mut files = Vec::new();
    for loaded in loader.iter() {
        files.push(LoadedFile {
            path: loaded.path.clone(),
            source: loaded.source.clone(),
            module: loaded.module.clone(),
        });
    }
    Ok(files)
}

fn typecheck_all(files: &[LoadedFile]) -> Result<(), String> {
    let mut env = Env::new();
    for lf in files.iter().rev() {
        let mut checker = TypeChecker::new(&mut env);
        checker.check_module(&lf.module)
            .map_err(|e| format!("{}", e.with_source(&lf.source).with_file(&lf.path.to_string_lossy())))?;
    }
    Ok(())
}

fn run_program(path: &PathBuf, watch: bool, engine: Engine) -> Result<String, String> {
    if watch {
        return watch::run(path, engine);
    }
    let files = load(path)?;
    typecheck_all(&files)?;

    if engine == Engine::Native {
        if let Some(exe) = native_executable(&files) {
            let status = std::process::Command::new(&exe).status().map_err(|e| format!("Failed to start {}: {}", exe.display(), e))?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
            return Ok(String::new());
        }
    }

    print!("{}", interpret_files(path, &files, true)?);
    Ok(String::new())
}

fn interpret_files(path: &Path, files: &[LoadedFile], tui_mode: bool) -> Result<String, String> {
    let mut interp = Interpreter::new();
    interp.tui_mode = tui_mode;
    interp.source_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    for lf in files {
        interp.load_module(&lf.module);
    }
    interp.run_main()
        .map_err(|e| format!("{}", e.with_source(&files[0].source).with_file(&files[0].path.to_string_lossy())))
}

#[cfg(test)]
pub(crate) fn interpret_project(path: &Path) -> Result<String, String> {
    let files = load(&path.to_path_buf())?;
    typecheck_all(&files)?;
    interpret_files(path, &files, false)
}

fn native_executable(files: &[LoadedFile]) -> Option<PathBuf> {
    let sources: Vec<native::Source> = files
        .iter()
        .map(|f| native::Source { path: &f.path, text: &f.source, module: &f.module })
        .collect();
    native::executable(&sources, &crate::codegen::backend::LlvmBackend)
}

pub(super) struct Launch {
    pub command: std::process::Command,
    pub sources: Vec<PathBuf>,
}

pub(super) fn prepare_launch(path: &PathBuf, engine: Engine) -> Result<Launch, String> {
    let files = load(path)?;
    typecheck_all(&files)?;
    let sources = files.iter().map(|f| f.path.clone()).collect();
    let native = if engine == Engine::Native { native_executable(&files) } else { None };
    let command = match native {
        Some(exe) => std::process::Command::new(exe),
        None => interpreter_command(path)?,
    };
    Ok(Launch { command, sources })
}

fn interpreter_command(path: &Path) -> Result<std::process::Command, String> {
    let yidi = std::env::current_exe().map_err(|e| format!("Cannot locate the yidi binary: {}", e))?;
    let mut command = std::process::Command::new(yidi);
    command.arg("run").arg(path).arg("--interp");
    Ok(command)
}

pub(crate) fn build_program(path: &PathBuf, watch: bool, backend: &dyn CodegenBackend, show_info: bool) -> Result<String, String> {
    let files = load(path)?;
    typecheck_all(&files)?;

    if show_info {
        let hw = hardware::detect();
        println!("{}", hw);
    }

    let hw = hardware::detect();
    let output_path = path.with_extension("");

    let modules: Vec<&Module> = files.iter().map(|f| &f.module).collect();
    let file_paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    backend.compile_with_paths(&modules, &file_paths, &output_path, &hw)
        .map_err(|e| e.to_string())?;

    if watch {
        return watch::rebuild(path, backend, show_info);
    }

    Ok(String::new())
}

#[cfg(test)]
fn load_module(source: &str) -> Module {
    crate::syntax::ast::reset_ids();
    crate::syntax::parser::Parser::parse(source).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::backend::MockBackend;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_flags_select_version_command() {
        for flag in ["--version", "-V", "version"] {
            assert!(matches!(parse_from(&args(&["yidi", flag])), Command::Version));
        }
    }

    #[test]
    fn package_commands_are_parsed() {
        assert!(matches!(parse_from(&args(&["yidi", "init"])), Command::Init(None)));
        assert!(matches!(parse_from(&args(&["yidi", "init", "app"])), Command::Init(Some(n)) if n == "app"));
        assert!(matches!(parse_from(&args(&["yidi", "remove", "greet"])), Command::Remove(n) if n == "greet"));
    }

    #[test]
    fn upgrade_accepts_self_and_check() {
        assert!(matches!(parse_from(&args(&["yidi", "upgrade"])), Command::Upgrade(false)));
        assert!(matches!(parse_from(&args(&["yidi", "upgrade", "--self"])), Command::Upgrade(false)));
        assert!(matches!(parse_from(&args(&["yidi", "upgrade", "--check"])), Command::Upgrade(true)));
    }

    #[test]
    fn version_output_names_binary_and_crate_version() {
        let out = execute(Command::Version).unwrap();
        assert_eq!(out, format!("yidi {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn bare_path_still_runs_file() {
        assert!(matches!(parse_from(&args(&["yidi", "hello.yk"])), Command::Run(ref f, false, Engine::Native) if f == "hello.yk"));
    }

    #[test]
    fn native_is_the_default_and_interp_opts_out() {
        assert!(matches!(parse_from(&args(&["yidi", "run", "a.yk"])), Command::Run(_, false, Engine::Native)));
        assert!(matches!(parse_from(&args(&["yidi", "run", "a.yk", "--interp"])), Command::Run(_, false, Engine::Interp)));
        assert!(matches!(parse_from(&args(&["yidi", "a.yk", "--interp", "--watch"])), Command::Run(_, true, Engine::Interp)));
    }

    #[test]
    fn build_program_with_mock_backend() {
        let dir = std::env::temp_dir().join("yidi_test_cli");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("test_build.yk");
        std::fs::write(&file_path, "fn main() { print(\"hello\"); }").unwrap();

        let backend = MockBackend::new();
        build_program(&file_path, false, &backend, false).unwrap();

        let outputs = backend.output_paths.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_program_fails_on_bad_source() {
        let dir = std::env::temp_dir().join("yidi_test_cli_fail");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("bad.yk");
        std::fs::write(&file_path, "fn main() { syntax error }").unwrap();

        let backend = MockBackend::new();
        let result = build_program(&file_path, false, &backend, false);
        assert!(result.is_err());

        let outputs = backend.output_paths.lock().unwrap();
        assert_eq!(outputs.len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typecheck_all_accumulates_errors() {
        let files = vec![
            LoadedFile {
                path: PathBuf::from("test.yk"),
                source: "fn main() { x: int = \"wrong_type\"; }".into(),
                module: load_module("fn main() { x: int = \"wrong_type\"; }"),
            },
        ];
        let result = typecheck_all(&files);
        assert!(result.is_err());
    }

    #[test]
    fn load_valid_source_succeeds() {
        let dir = std::env::temp_dir().join("yidi_test_load");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("simple.yk");
        std::fs::write(&file_path, "fn main() {}").unwrap();

        let result = load(&file_path);
        assert!(result.is_ok());
        let files = result.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].module.items.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn end_to_end_full_pipeline_ok() {
        let dir = std::env::temp_dir().join("yidi_e2e_ok");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path,
            "fn add(a: int, b: int) -> int { return a + b; }
             fn main() { x: int = add(1, 2); print(x); }"
        ).unwrap();

        let backend = MockBackend::new();
        let result = build_program(&file_path, false, &backend, false);
        assert!(result.is_ok());

        let outputs = backend.output_paths.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn end_to_end_with_struct_and_field() {
        let dir = std::env::temp_dir().join("yidi_e2e_struct");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("app.yk");
        std::fs::write(&file_path,
            "struct Point { x: int, y: int }
             fn main() { p: Point = Point { x: 10, y: 20 }; print(p.x); }"
        ).unwrap();

        let backend = MockBackend::new();
        let result = build_program(&file_path, false, &backend, false);
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn end_to_end_with_fn_call() {
        let dir = std::env::temp_dir().join("yidi_e2e_fncall");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("app.yk");
        std::fs::write(&file_path,
            "fn double(x: int) -> int { return x * 2; }
             fn main() { result: int = double(5); print(result); }"
        ).unwrap();

        let backend = MockBackend::new();
        build_program(&file_path, false, &backend, false).unwrap();

        let outputs = backend.output_paths.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn end_to_end_type_error_rejected() {
        let dir = std::env::temp_dir().join("yidi_e2e_typeerr");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("bad.yk");
        std::fs::write(&file_path,
            "fn main() { x: int = \"not a number\"; }"
        ).unwrap();

        let backend = MockBackend::new();
        let result = build_program(&file_path, false, &backend, false);
        assert!(result.is_err());

        let outputs = backend.output_paths.lock().unwrap();
        assert_eq!(outputs.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn end_to_end_generic_class() {
        let dir = std::env::temp_dir().join("yidi_e2e_genclass");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("app.yk");
        std::fs::write(&file_path,
            "class Box<T> { val: T; fn get(self) -> T { return self.val; } }
             fn main() { b: Box<int> = Box { val: 42 }; print(b.get()); }"
        ).unwrap();

        let backend = MockBackend::new();
        let result = build_program(&file_path, false, &backend, false);
        assert!(result.is_ok());

        let outputs = backend.output_paths.lock().unwrap();
        assert_eq!(outputs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_show_info() {
        let result = show_info();
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.contains("Yidika Hardware Info"));
    }
}
