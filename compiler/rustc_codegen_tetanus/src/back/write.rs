use crate::TetanusCodegenBackend;
use rustc_codegen_ssa::CompiledModule;
// use rustc_codegen_ssa::ModuleKind;
use rustc_codegen_ssa::ModuleCodegen;
use rustc_codegen_ssa::back::write::{CodegenContext, ModuleConfig};
use rustc_session::config::OutputType;
use std::fs::File;
use std::io::Write;
// use super::elf_builder::ElfBuilder;
use std::process::Command;

pub fn codegen(
    cgcx: &CodegenContext<TetanusCodegenBackend>,
    module: ModuleCodegen<
        <TetanusCodegenBackend as rustc_codegen_ssa::traits::WriteBackendMethods>::Module,
    >,
    _config: &ModuleConfig,
) -> CompiledModule {
    let path = cgcx.output_filenames.temp_path_for_cgu(
        OutputType::Object,
        &module.name,
        cgcx.invocation_temp.as_deref(),
    );
    
    eprintln!("{:?}", path);
    let assem_file_name = path.to_str().unwrap().to_owned() + ".s";
    let mut file = File::create(&assem_file_name).unwrap();

    file.write_all(b"main:\n li	a0,-1\n.LM4:\n ret\n").unwrap();
    Command::new("riscv64-unknown-elf-gcc")
        .arg("-c")
        .arg(&assem_file_name)
        .arg("-o")
        .arg(path.to_str().unwrap())
        .spawn()
        .unwrap()
        .wait()
        .unwrap();

    module.into_compiled_module(
        false,
        false,
        false,
        true,
        false,
        &cgcx.output_filenames,
        cgcx.invocation_temp.as_deref(),
    )
}
