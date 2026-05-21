use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::interpret::Interpreter;
use crate::module::ModuleLoader;
use crate::semantic::env::Env;
use crate::semantic::typeck::TypeChecker;
use crate::syntax::ast::{self, Module};
use crate::syntax::parser::Parser;

pub enum Command {
    Run(String, bool),
    Build(String, bool),
    Add(String),
    Sync,
}

pub fn parse_args() -> Command {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage:");
        eprintln!("  yidi <file>              Run a .yk file (interpreter)");
        eprintln!("  yidi <file> --watch      Run file and watch for changes");
        eprintln!("  yidi build <file>        Build a .yk file");
        eprintln!("  yidi build <file> --watch Rebuild on file change");
        eprintln!("  yidi add <package>        Add a dependency");
        eprintln!("  yidi sync               Sync dependencies");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "build" => {
            if args.len() < 3 { eprintln!("Usage: yidi build <file>"); std::process::exit(1); }
            let watch = args.iter().any(|a| a == "--watch");
            Command::Build(args[2].clone(), watch)
        }
        "add" => {
            if args.len() < 3 { eprintln!("Usage: yidi add <package>"); std::process::exit(1); }
            Command::Add(args[2].clone())
        }
        "sync" => Command::Sync,
        _ => {
            let watch = args.iter().any(|a| a == "--watch");
            Command::Run(args[1].clone(), watch)
        }
    }
}

pub fn execute(cmd: Command) -> Result<String, String> {
    match cmd {
        Command::Run(f, watch) => run_program(&PathBuf::from(f), watch),
        Command::Build(f, watch) => build_program(&PathBuf::from(f), watch),
        Command::Add(pkg) => add_package(&pkg),
        Command::Sync => sync_packages(),
    }
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
    for path in loader.iter() {
        let source = fs::read_to_string(path)
            .map_err(|e| format!("IO error: {}", e))?;
        ast::reset_ids();
        let module = Parser::parse(&source)
            .map_err(|e| format!("{}", e.with_source(&source).with_file(&path.to_string_lossy())))?;
        files.push(LoadedFile { path: path.clone(), source, module });
    }
    Ok(files)
}

fn typecheck_all(files: &[LoadedFile]) -> Result<(), String> {
    let mut env = Env::new();
    for lf in files {
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

fn build_program(path: &PathBuf, watch: bool) -> Result<String, String> {
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    let files = load(path)?;
    typecheck_all(&files)?;
    let llvm_ir = crate::codegen::llvm::compile_to_llvm(&files[0].module);
    let output_path = path.with_extension("");
    crate::codegen::llvm::compile_to_exe(&llvm_ir, &output_path)
        .map_err(|e| e.to_string())?;
    println!("✅ Build OK: {} -> {}.exe ({} files)", file, output_path.to_string_lossy(), files.len());

    if watch {
        watch_and_build(path)?;
    }

    Ok(String::new())
}

fn watch_and_build(path: &PathBuf) -> Result<(), String> {
    let mut last = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok();
    println!("👀 Watching {} for changes...", path.display());
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if last.map_or(true, |l| modified > l) {
                    last = Some(modified);
                    println!("🔄 Change detected, rebuilding...");
                    let _ = build_program(path, true);
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
