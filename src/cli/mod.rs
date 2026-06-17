use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::codegen::backend::CodegenBackend;
use crate::hardware;
use crate::interpret::Interpreter;
use crate::module::ModuleLoader;
use crate::semantic::env::Env;
use crate::semantic::typeck::TypeChecker;
use crate::syntax::ast::Module;

pub enum Command {
    Run(String, bool),
    Build(String, bool),
    BuildWithInfo(String, bool),
    Add(String),
    Sync,
    Info,
}

pub fn parse_args() -> Command {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage:");
        eprintln!("  yidi <file>              Run a .yk file (interpreter)");
        eprintln!("  yidi run <file>          Run a .yk file (interpreter)");
        eprintln!("  yidi <file> --watch      Run file and watch for changes");
        eprintln!("  yidi build <file>        Build a .yk file");
        eprintln!("  yidi build <file> --watch Rebuild on file change");
        eprintln!("  yidi info                Show hardware info");
        eprintln!("  yidi add <package>        Add a dependency");
        eprintln!("  yidi sync               Sync dependencies");
        std::process::exit(1);
    }

    match args[1].as_str() {
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
            let watch = args.iter().any(|a| a == "--watch");
            Command::Run(args[2].clone(), watch)
        }
        "add" => {
            if args.len() < 3 { eprintln!("Usage: yidi add <package>"); std::process::exit(1); }
            Command::Add(args[2].clone())
        }
        "sync" => Command::Sync,
        _ => {
            // backward compatibility: treat bare path as "run"
            let watch = args.iter().any(|a| a == "--watch");
            Command::Run(args[1].clone(), watch)
        }
    }
}

pub fn execute(cmd: Command) -> Result<String, String> {
    match cmd {
        Command::Run(f, watch) => run_program(&PathBuf::from(f), watch),
        Command::Build(f, watch) => build_program(&PathBuf::from(f), watch, &crate::codegen::backend::LlvmBackend, false),
        Command::BuildWithInfo(f, watch) => build_program(&PathBuf::from(f), watch, &crate::codegen::backend::LlvmBackend, true),
        Command::Info => show_info(),
        Command::Add(pkg) => add_package(&pkg),
        Command::Sync => sync_packages(),
    }
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

fn run_program(path: &PathBuf, watch: bool) -> Result<String, String> {
    let files = load(path)?;
    typecheck_all(&files)?;

    let mut interp = Interpreter::new();
    interp.tui_mode = true;
    interp.source_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    for lf in &files {
        interp.load_module(&lf.module);
    }
    let output = interp.run_main()
        .map_err(|e| format!("{}", e.with_source(&files[0].source).with_file(&files[0].path.to_string_lossy())))?;

    print!("{}", output);

    if watch {
        watch_and_run(path)?;
    }

    Ok(String::new())
}

fn watch_and_run(path: &PathBuf) -> Result<(), String> {
    let mut last = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if last.map_or(true, |l| modified > l) {
                    last = Some(modified);
                    let _ = run_program(path, true);
                }
            }
        }
    }
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
        watch_and_build(path)?;
    }

    Ok(String::new())
}

fn watch_and_build(path: &PathBuf) -> Result<(), String> {
    let mut last = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if last.map_or(true, |l| modified > l) {
                    last = Some(modified);
                    let _ = build_program(path, true, &crate::codegen::backend::LlvmBackend, false);
                }
            }
        }
    }
}

fn add_package(name: &str) -> Result<String, String> {
    crate::package::add(name)
}

fn sync_packages() -> Result<String, String> {
    crate::package::sync()
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
