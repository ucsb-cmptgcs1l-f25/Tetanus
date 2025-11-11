use rustc_codegen_ssa::CompiledModule;
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_codegen_ssa::CrateInfo;
use rustc_codegen_ssa::CodegenResults;
use rustc_data_structures::fx::FxIndexMap;
use rustc_session::Session;
use rustc_middle::ty::TyCtxt;
use std::thread::JoinHandle;
use std::path::PathBuf;
use rustc_session::config::OutputFilenames;
use rustc_middle::ty::Instance;
use rustc_middle::mir::mono::MonoItem;
use rustc_middle::mir::{TerminatorKind, StatementKind};
use rustc_codegen_ssa::ModuleKind;
use rustc_data_structures::stable_hasher::{StableHasher, HashStable};
use rustc_session::config::OutputType;
use rustc_hir::Mutability;
use rustc_middle::ty::{TyKind};
// use rustc_span::sym::module;

use std::fs::File;
use std::io::Write;

use crate::CPU_NAME;
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) struct ModuleCodegenResult {
    module_regular: CompiledModule,
    module_global_asm: Option<CompiledModule>,
    existing_work_product: Option<(WorkProductId, WorkProduct)>,
}

#[allow(dead_code)]
pub(crate) enum OngoingModuleCodegen {
    Sync(Result<ModuleCodegenResult, String>),
    Async(JoinHandle<Result<ModuleCodegenResult, String>>),
}

#[allow(dead_code)]
pub(crate) struct OngoingCodegen {
    pub modules: Vec<OngoingModuleCodegen>,
    pub allocator_module: Option<CompiledModule>,
    pub crate_info: CrateInfo,
}

impl OngoingCodegen {
    pub(crate) fn join(
        self,
        _sess: &Session,
        // outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        (
            CodegenResults {
                modules: vec![],
                allocator_module: None,
                crate_info: self.crate_info,
            },
            FxIndexMap::default(),
        )
    }
}

impl<HCX> HashStable<HCX> for OngoingModuleCodegen {
    fn hash_stable(&self, _: &mut HCX, _: &mut StableHasher) {
        // do nothing
    }
}

#[derive(Debug)]
pub(crate) struct GlobalAsmConfig {
    _assembler: PathBuf,
    _target: String,
    pub(crate) _output_filenames: Arc<OutputFilenames>,
}

impl GlobalAsmConfig {
    pub(crate) fn new(tcx: TyCtxt<'_>) -> Self {
        GlobalAsmConfig {
            _assembler: crate::toolchain::get_toolchain_binary(tcx.sess, "as"),
            _target: "riscv64gc-unknown-linux-gnu".to_string(),
            _output_filenames: tcx.output_filenames(()).clone(),
        }
    }
}

/// Create and start generation for cgus
/// this is stolen from cranelift
pub(crate) fn run_aot(tcx: TyCtxt<'_>) -> Box<OngoingCodegen> {
    let target_cpu = CPU_NAME.to_string();

    let cgus = if tcx.sess.opts.output_types.should_codegen() {
        tcx.collect_and_partition_mono_items(()).codegen_units
    } else {
        // If only `--emit metadata` is used, we shouldn't perform any codegen.
        // Also `tcx.collect_and_partition_mono_items` may panic in that case.
        return Box::new(OngoingCodegen {
            modules: vec![],
            allocator_module: None,
            crate_info: CrateInfo::new(tcx, target_cpu),
            // concurrency_limiter: ConcurrencyLimiter::new(0),
        });
    };

    if tcx.dep_graph.is_fully_enabled() {
        for cgu in cgus {
            tcx.ensure_ok().codegen_unit(cgu.name());
        }
    }

    let global_asm_config = Arc::new(GlobalAsmConfig::new(tcx));

    let mut modules = vec![];
    for cgu in cgus {
        modules.push(start_module_codegen(tcx, (global_asm_config.clone(), cgu.name())))
    }

    Box::new(OngoingCodegen {
        modules,
        allocator_module: None, // allocator_module,
        crate_info: CrateInfo::new(tcx, target_cpu),
        // concurrency_limiter: concurrency_limiter.0,
    })
}

/// Generated assembly for a cgu
fn start_module_codegen(
    tcx: TyCtxt<'_>,
    (_global_asm_config, cgu_name): (
        Arc<GlobalAsmConfig>,
        rustc_span::Symbol,
        // ConcurrencyLimiterToken,
    ),
) -> OngoingModuleCodegen {
    eprintln!("running module codegen for {}", cgu_name);
    let cgu = tcx.codegen_unit(cgu_name);
    let mono_items = cgu.items_in_deterministic_order(tcx);
    eprintln!("mono items len {}", mono_items.len());
    let mut asm = String::new();
    for (i, item) in mono_items.into_iter().enumerate() {
        asm += match item {
            (MonoItem::Fn(inst), _) => codegen_function(tcx, tcx.symbol_name(inst).name, inst),
            _ => { eprintln!("mono item {}: {:?}", i, item); String::new() },
        }.as_str();
    }

    let path = tcx.output_filenames(()).temp_path_for_cgu(
        OutputType::Object,
        cgu.name().to_string().as_str(),
        tcx.sess.invocation_temp.as_deref(),
    );
    let assem_file_name = path.to_str().unwrap().to_owned() + ".s";
    let mut file = File::create(&assem_file_name).unwrap();
    file.write_all(asm.as_bytes()).unwrap();

    OngoingModuleCodegen::Sync(Ok(ModuleCodegenResult{
        module_regular: CompiledModule {
            name: format!("{cgu_name}.asm"),
            kind: ModuleKind::Regular,
            object: Some(path),
            dwarf_object: None,
            bytecode: None,
            assembly: None,// assem_file_name,
            llvm_ir: None,
            links_from_incr_cache: Vec::new(),
        },
        module_global_asm: None,
        existing_work_product: None,
    }))
}

fn codegen_function<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbol_name: &str,
    // module: &mut dyn Module,
    inst: Instance<'tcx>,
) -> String {
    // eprintln!("name: {}", symbol_name);
    let mir = tcx.instance_mir(inst.def);

    // eprintln!("mir: {:?}", mir);

    let mut asm = String::new();
    asm += symbol_name;
    asm += ":\n";
    // calling convention
    // we save everything on the stack rn bc i dont wanna get too fancy
    let mut stack_offset: isize = 0;
    // TODO save save registers
    // (declartion, type size in bytes)
    let mut locals = vec![];
    for decl in &mir.local_decls {
        // eprintln!("{:?}\n", decl);
        locals.push((decl, local_size(decl.ty.kind(), tcx).unwrap_or(1) as isize));
    }

    for (_i, local) in locals.into_iter().enumerate() {
        // Push 0 onto stack
        asm += format!("\tsd\tzero, {}(sp) ", -local.1).as_str();
        asm += "; let";
        if local.0.mutability == Mutability::Mut { asm += " mut"; }
        asm += format!(" {:?}", local.0.ty).as_str();
        asm += "\n";
        // Update stack pointer
        stack_offset -= local.1;
        asm += format!("\taddi\tsp, sp, -{}\n", local.1).as_str();
    }

    for (id, bb) in (*mir.basic_blocks).into_iter().enumerate() {
        asm += format!("{:?}:\n", id).as_str();
        // generate statements
        for statement in &bb.statements {
            match statement.kind {
                StatementKind::StorageLive(_) | StatementKind::StorageDead(_) => {},
                _ => asm += format!("\t; TODO: {:?}\n", statement.kind).as_str(),
            }
        }
        // generate terminator
        match bb.terminator().kind {
            TerminatorKind::Goto{ target } => asm += format!("\tj {:?}\n", target).as_str(),
            TerminatorKind::Return => asm += "\tj end\n",
            _ => asm += format!("\t; TODO: {:?}\n\n", bb.terminator().kind).as_str(),
        }
    }

    asm += "end:\n";
    asm += format!("\taddi\tsp, sp, {}", -stack_offset).as_str();
    asm += "\nret\n\n";
    return asm;
}

fn local_size<'tcx>(ty: &TyKind<'tcx>, tcx: TyCtxt<'tcx>) -> Result<usize, String> {
    match ty {
        TyKind::Bool | TyKind::Char => Ok(1),
        TyKind::Int(ity) => Ok(ity.bit_width().unwrap_or(64) as usize / 8),
        TyKind::Uint(uity) => Ok(uity.bit_width().unwrap_or(64) as usize / 8),
        TyKind::Float(fty) => Ok(fty.bit_width() as usize / 8),
        // ptrs
        TyKind::RawPtr(..) => Ok(8), // TODO do we handle rv32
        TyKind::Ref(..) => Ok(8),
        TyKind::FnPtr(..) => Ok(8), // is this right
        // collectiony things
        TyKind::Array(t, n) => local_size(t.kind(), tcx).map(|s| s * n.try_to_target_usize(tcx).unwrap() as usize),
        // TyKind::Tuple(tys) => tys.iter().map(|t| local_size(t.kind(), tcx)).sum(),

        _ => Err(format!("unknown type: {:?}", ty)),
    }
}

#[test]
pub fn test_type_sizes() {
    assert_equal!(
        aot::local_size(TyKind::Bool),
        Ok(1)
    );

    assert_equal!(
        aot::local_size(TyKind::Char),
        Ok(1)
    );

    assert_equal!(
        aot::local_size(TyKind::Int(IntTy::I32)),
        Ok(4)
    );

    assert_equal!(
        aot::local_size(TyKind::Uint(UintTy::U32)),
        Ok(4)
    );

    assert_equal!(
        aot::local_size(TyKind::Float(FloatTy::F32)),
        Ok(4)
    );
}