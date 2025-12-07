//! The Rust compiler.
//!
//! # Note
//!
//! This API is completely unstable and subject to change.

// tidy-alphabetical-start

// tidy-alphabetical-end

// #![deny(warnings)]
#![feature(rustc_private)]
#![feature(box_patterns)]

#[allow(unused_extern_crates)]
extern crate rustc_driver;
// #[macro_use]
// extern crate tracing;
extern crate rustc_ast;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_errors;
extern crate rustc_fluent_macro;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use std::any::Any;
use std::fmt::Write;
use std::path::PathBuf;
use std::string::String;
use std::sync::Arc;

use rustc_ast::expand::allocator::AllocatorKind;
use rustc_ast::expand::autodiff_attrs::AutoDiffItem;
use rustc_codegen_ssa::back::lto::{SerializedModule, ThinModule};
use rustc_codegen_ssa::back::write::{
    CodegenContext, FatLtoInput, ModuleConfig, TargetMachineFactoryFn,
};
use rustc_codegen_ssa::traits::*;
use rustc_codegen_ssa::{CodegenResults, CompiledModule, ModuleCodegen, TargetConfig};
use rustc_data_structures::fx::FxIndexMap;
use rustc_errors::{DiagCtxtHandle, FatalError};
#[allow(unused_imports)]
use rustc_hir::def_id::LOCAL_CRATE;
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_middle::ty::TyCtxt;
use rustc_session::Session;
use rustc_session::config::{OptLevel, OutputFilenames, PrintRequest};
use rustc_span::Symbol;

mod back;
mod codegen;
mod toolchain;

rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

#[derive(Clone)]
pub struct TetanusCodegenBackend(());

static CPU_NAME: &str = "riscv";

impl CodegenBackend for TetanusCodegenBackend {
    fn locale_resource(&self) -> &'static str {
        crate::DEFAULT_LOCALE_RESOURCE
    }

    fn init(&self, sess: &Session) {
        if sess.target.arch != "riscv64" {
            panic!("invalid arch {:?}", sess.target.arch);
        }
    }

    fn print(&self, req: &PrintRequest, out: &mut String, _sess: &Session) {
        writeln!(out, "havent done print() yet").unwrap();
        writeln!(out, "req: {:?}", req.kind).unwrap();
    }

    fn target_config(&self, _sess: &Session) -> TargetConfig {
        TargetConfig {
            target_features: vec![],
            unstable_target_features: vec![],
            has_reliable_f16: false,
            has_reliable_f16_math: false,
            has_reliable_f128: false,
            has_reliable_f128_math: false,
        }
    }

    fn join_codegen(
        &self,
        ongoing_codegen: Box<dyn Any>,
        sess: &Session,
        _outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        let (codegen_results, work_products) = ongoing_codegen
            .downcast::<codegen::aot::OngoingCodegen>()
            .expect("Expected Tetanus's OngoingCodegen, found Box<Any>")
            .join(sess);
        (codegen_results, work_products)
    }

    fn codegen_crate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Box<dyn Any> {
        codegen::aot::run_aot(tcx)
    }
}

impl ExtraBackendMethods for TetanusCodegenBackend {
    fn codegen_allocator<'tcx>(
        &self,
        _tcx: TyCtxt<'tcx>,
        _module_name: &str,
        _kind: AllocatorKind,
        _alloc_error_handler_kind: AllocatorKind,
    ) -> Self::Module {
        Self::Module {}
    }

    /// This generates the codegen unit and returns it along with
    /// a `u64` giving an estimate of the unit's processing cost.
    fn compile_codegen_unit(
        &self,
        _tcx: TyCtxt<'_>,
        cgu_name: Symbol,
    ) -> (ModuleCodegen<Self::Module>, u64) {
        (ModuleCodegen::<Self::Module>::new_regular(cgu_name.to_string(), Self::Module {}), 64)
    }

    fn target_machine_factory(
        &self,
        _sess: &Session,
        _opt_level: OptLevel,
        _target_features: &[String],
    ) -> TargetMachineFactoryFn<Self> {
        Arc::new(|_| Ok(()))
    }

    // fn supports_parallel(&self) -> bool {
    //     false
    // }
}

impl WriteBackendMethods for TetanusCodegenBackend {
    type Module = ModuleTetanus;
    type ModuleBuffer = back::lto::ModuleBuffer;
    type TargetMachine = ();
    type TargetMachineError = ();
    type ThinData = back::lto::ThinData;
    type ThinBuffer = back::lto::ThinBuffer;

    /// Performs fat LTO by merging all modules into a single one, running autodiff
    /// if necessary and running any further optimizations
    fn run_and_optimize_fat_lto(
        _cgcx: &CodegenContext<Self>,
        _exported_symbols_for_lto: &[String],
        _each_linked_rlib_for_lto: &[PathBuf],
        _modules: Vec<FatLtoInput<Self>>,
        _diff_fncs: Vec<AutoDiffItem>,
    ) -> Result<ModuleCodegen<Self::Module>, FatalError> {
        panic!("not implemented yet!");
    }

    /// Performs thin LTO by performing necessary global analysis and returning two
    /// lists, one of the modules that need optimization and another for modules that
    /// can simply be copied over from the incr. comp. cache.
    fn run_thin_lto(
        _cgcx: &CodegenContext<Self>,
        _exported_symbols_for_lto: &[String],
        _each_linked_rlib_for_lto: &[PathBuf],
        _modules: Vec<(String, Self::ThinBuffer)>,
        _cached_modules: Vec<(SerializedModule<Self::ModuleBuffer>, WorkProduct)>,
    ) -> Result<(Vec<ThinModule<Self>>, Vec<WorkProduct>), FatalError> {
        panic!("not implemented yet!");
    }
    fn print_pass_timings(&self) {
        panic!("not implemented yet!");
    }
    fn print_statistics(&self) {
        panic!("not implemented yet!");
    }
    fn optimize(
        _cgcx: &CodegenContext<Self>,
        _dcx: DiagCtxtHandle<'_>,
        _module: &mut ModuleCodegen<Self::Module>,
        _config: &ModuleConfig,
    ) -> Result<(), FatalError> {
        Ok(())
    }
    fn optimize_thin(
        _cgcx: &CodegenContext<Self>,
        _thin: ThinModule<Self>,
    ) -> Result<ModuleCodegen<Self::Module>, FatalError> {
        panic!("not implemented yet!");
    }
    fn codegen(
        cgcx: &CodegenContext<Self>,
        module: ModuleCodegen<Self::Module>,
        config: &ModuleConfig,
    ) -> Result<CompiledModule, FatalError> {
        Ok(back::write::codegen(cgcx, module, config))
    }
    fn prepare_thin(
        _module: ModuleCodegen<Self::Module>,
        _want_summary: bool,
    ) -> (String, Self::ThinBuffer) {
        panic!("not implemented yet!");
    }
    fn serialize_module(_module: ModuleCodegen<Self::Module>) -> (String, Self::ModuleBuffer) {
        panic!("not implemented yet!");
    }
}

pub struct ModuleTetanus {}

/// This is the entrypoint for a hot plugged rustc_codegen_tetanus
#[unsafe(no_mangle)]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(TetanusCodegenBackend(()))
}
