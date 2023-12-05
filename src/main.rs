use calyx_backend::Backend;
use calyx_opt::pass_manager::PassManager;
use fil_ir as ir;
use filament::ir_passes::BuildDomination;
use filament::{cmdline, ir_passes as ip, resolver::Resolver};
use filament::{log_pass, log_time, pass_pipeline};
use std::io::Write;

// Prints out the interface for main component in the input program.
fn run(opts: &cmdline::Opts) -> Result<(), u64> {
    // enable tracing
    env_logger::Builder::from_default_env()
        .format_timestamp(None)
        .format_module_path(false)
        .format_target(false)
        .filter_level(opts.log_level)
        .target(env_logger::Target::Stderr)
        .init();

    let mut resolver = Resolver::from(opts);
    let ns = match resolver.parse_namespace() {
        Ok(mut ns) => {
            ns.toplevel = opts.toplevel.clone();
            ns
        }
        Err(e) => {
            eprintln!("Error: {e:?}");
            return Err(1);
        }
    };

    if let Some(dep_file) = opts.dump_dep_file.as_ref() {
        if let Some(out) = &opts.out {
            let mut deps = resolver
                .already_imported
                .iter()
                .map(|imp| imp.display().to_string())
                .chain(ns.externs.iter().map(|ext| ext.path.to_string()))
                .collect::<Vec<_>>();
            let last = deps.pop();

            let mut file = std::fs::File::create(dep_file).unwrap();
            write!(&mut file, "{}: ", out.display()).unwrap();
            for dep in deps {
                writeln!(&mut file, "{dep} \\").unwrap();
            }
            if let Some(dep) = last {
                writeln!(&mut file, "{dep}").unwrap();
            }
        }
    }

    // Initialize the generator
    let mut gen_exec = if ns.requires_gen() {
        if opts.out_dir.is_none()
            && matches!(opts.backend, cmdline::Backend::Calyx)
        {
            log::warn!(concat!(
                "Generated calyx program will NOT compile because it depends ",
                "on generated files. Please provide an output directory using ",
                "`--out-dir <dir>` to store the generated files."
            ))
        }
        Some(ns.init_gen(opts.out_dir.clone()))
    } else {
        None
    };

    // Transform AST to IR
    let mut ir = log_pass! { opts; ir::transform(ns)?, "astconv" };
    pass_pipeline! {opts, ir;
        ip::BuildDomination,
        ip::TypeCheck,
        ip::IntervalCheck,
        ip::PhantomCheck,
        ip::Assume
    }
    if !opts.unsafe_skip_discharge {
        pass_pipeline! {opts, ir; ip::Discharge }
    }
    pass_pipeline! { opts, ir;
        BuildDomination
    };
    ir = log_pass! { opts; ip::Monomorphize::transform(&ir, &mut gen_exec), "monomorphize"};
    pass_pipeline! { opts, ir;
        ip::Simplify,
        ip::AssignCheck,
        ip::BundleElim,
        ip::AssignCheck
    }

    // Return early if we're asked to dump the interface
    if let Some(dump_file) = opts.dump_interface_file.as_ref() {
        let file = std::fs::File::create(dump_file).unwrap();
        ip::DumpInterface::write(&ir, file);
        // return Ok(());
    }

    // Return if we are only checking
    if opts.check {
        return Ok(());
    }
    let calyx = log_time!(
        ip::Compile::compile(ir, opts.disable_slow_fsms, opts.preserve_names),
        "compile"
    );
    match opts.backend {
        cmdline::Backend::Verilog => {
            gen_verilog(calyx).unwrap();
        }
        cmdline::Backend::Calyx => {
            let out = &mut std::io::stdout();
            calyx_ir::Printer::write_context(&calyx, false, out).unwrap();
        }
    }

    // Drop the generator executor after the execution finishes to ensure that
    // Calyx has access to the generated file.
    drop(gen_exec);
    Ok(())
}

fn gen_verilog(mut ctx: calyx_ir::Context) -> Result<(), calyx_utils::Error> {
    let pm = PassManager::default_passes()?;
    let backend_conf = calyx_ir::BackendConf {
        synthesis_mode: false,
        enable_verification: false,
        flat_assign: true,
    };
    ctx.bc = backend_conf;
    pm.execute_plan(
        &mut ctx,
        &["all".to_string()],
        &["canonicalize".to_string()],
        false,
    )?;

    let file = calyx_utils::OutputFile::Stdout;
    let backend = calyx_backend::VerilogBackend;
    backend.run(ctx, file)
}

fn main() {
    let opts: cmdline::Opts = argh::from_env();
    match run(&opts) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("Compilation failed with {err} errors.");
            if !opts.show_models {
                eprintln!("Run with --show-models to generate assignments for failing constraints.");
            }
            std::process::exit(1)
        }
    }
}
