// use rustc_codegen_ssa::ModuleCodegen;
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
use rustc_codegen_ssa::ModuleKind;
use rustc_data_structures::stable_hasher::{StableHasher, HashStable};
// use rustc_session::config::OutputType;

// use std::fs::File;

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
            (MonoItem::Fn(inst), _) => codegen_function(tcx.symbol_name(inst).name, inst),
            _ => { eprintln!("mono item {}: {:?}", i, item); String::new() },
        }.as_str();
    }

    // let path = cgcx.output_filenames.temp_path_for_cgu(
    //     OutputType::Object,
    //     &module.name,
    //     cgcx.invocation_temp.as_deref(),
    // );
    // let assem_file_name = path.to_str().unwrap().to_owned() + ".s";
    // let mut file = File::create(&assem_file_name).unwrap();
    // file.write_all(asm).unwrap();

    OngoingModuleCodegen::Sync(Ok(ModuleCodegenResult{
        module_regular: CompiledModule {
            name: format!("{cgu_name}.asm"),
            kind: ModuleKind::Regular,
            object: None,
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
    symbol_name: &str,
    // module: &mut dyn Module,
    inst: Instance<'tcx>,
) -> String {
    eprintln!("name: {}", symbol_name);
    let mut asm = String::new();
    asm += &(".".to_owned() + symbol_name);
    asm += ";";
    asm += format!("{:?}", inst).as_str();

    return asm;
}
