use std::fmt::Write as fmtWrite;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use rustc_codegen_ssa::{CodegenResults, CompiledModule, CrateInfo, ModuleKind};
use rustc_data_structures::fx::FxIndexMap;
use rustc_data_structures::stable_hasher::{HashStable, StableHasher};
use rustc_hir::Mutability;
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_middle::mir::mono::MonoItem;
use rustc_middle::mir::{
    BasicBlock, ConstOperand, LocalDecl, Operand, Place, Rvalue, TerminatorKind,
};
use rustc_middle::ty::{
    AdtDef,
    EarlyBinder,
    // ConstKind,
    GenericArgs,
    Instance,
    Ty,
    TyCtxt,
    TyKind,
    TypeFoldable,
    TypingEnv,
};
use rustc_session::Session;
use rustc_session::config::{OutputFilenames, OutputType};

// use rustc_span::DUMMY_SP;
use crate::CPU_NAME;

const COMMENT_CHAR: &str = "#";
const END_BB_IDX: u32 = 0x0FFFFFFF;

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
            CodegenResults { modules: vec![], allocator_module: None, crate_info: self.crate_info },
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
            _ => {
                eprintln!("mono item {}: {:?}", i, item);
                String::new()
            }
        }
        .as_str();
    }

    let path = tcx.output_filenames(()).temp_path_for_cgu(
        OutputType::Object,
        cgu.name().to_string().as_str(),
        tcx.sess.invocation_temp.as_deref(),
    );
    let assem_file_name = path.to_str().unwrap().to_owned() + ".s";
    let mut file = File::create(&assem_file_name).unwrap();
    file.write_all(asm.as_bytes()).unwrap();

    OngoingModuleCodegen::Sync(Ok(ModuleCodegenResult {
        module_regular: CompiledModule {
            name: format!("{cgu_name}.asm"),
            kind: ModuleKind::Regular,
            object: Some(path),
            dwarf_object: None,
            bytecode: None,
            assembly: None, // assem_file_name,
            llvm_ir: None,
            links_from_incr_cache: Vec::new(),
        },
        module_global_asm: None,
        existing_work_product: None,
    }))
}

pub fn monomorphize<'tcx, T>(tcx: TyCtxt<'tcx>, inst: Instance<'tcx>, value: T) -> T
where
    T: Copy + TypeFoldable<TyCtxt<'tcx>>,
{
    inst.instantiate_mir_and_normalize_erasing_regions(
        tcx,
        TypingEnv::fully_monomorphized(),
        EarlyBinder::bind(value),
    )
}

#[derive(Clone, Debug)]
struct Local<'tcx> {
    mono_ty: Ty<'tcx>,
    decl: &'tcx LocalDecl<'tcx>,
    stack_size: Result<usize, String>,
}

fn j_addr_for(cur_idx: usize, target: BasicBlock) -> String {
    let tar_idx = usize::from(target);
    format!("{}{}", tar_idx, if tar_idx > cur_idx { "f" } else { "b" })
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
    // Save ra and fp
    asm += format!("\tsd\tra, 8(sp)\n").as_str();
    asm += format!("\tsd\tfp, 16(sp)\n").as_str();
    stack_offset += 16;
    asm += format!("\tmv\tfp, sp\n").as_str();
    // TODO save saved registers

    // (declartion, type size in bytes, err msg)
    let mut locals = vec![];
    for decl in &mir.local_decls {
        // eprintln!("{:?}\n", decl);
        let mono_ty = monomorphize(tcx, inst, decl.ty);
        let size = local_size(mono_ty.kind(), tcx);
        locals.push(Local { mono_ty, decl, stack_size: size });
    }

    for (_i, local) in locals.iter().enumerate() {
        // Push 0 onto stack
        // TODO make this zero the right number of bytes
        asm += format!("\tsd\tzero, {}(sp) ", -(local.stack_size.clone().unwrap_or(0) as isize))
            .as_str();
        // comment what this is for
        write!(asm, "{COMMENT_CHAR} let").unwrap();
        if local.decl.mutability == Mutability::Mut {
            asm += " mut";
        }
        asm += format!(" {:?}", local.mono_ty).as_str();
        asm += "\n";
        if let Err(err) = local.stack_size.clone() {
            write!(asm, "{COMMENT_CHAR} ").unwrap();
            asm += err.as_str();
            asm += "\n";
        }
        // Update stack pointer
        stack_offset -= local.stack_size.clone().unwrap_or(0) as isize;
        asm += format!("\taddi\tsp, sp, -{}\n", local.stack_size.clone().unwrap_or(0)).as_str();
    }

    for (id, bb) in (*mir.basic_blocks).into_iter().enumerate() {
        asm += format!("{:?}:\n", id).as_str();
        // generate statements
        for statement in &bb.statements {
            use rustc_middle::mir::StatementKind::*;
            match &statement.kind {
                // We don't dynamically adjust the stack during a function call
                // So these are nops rn
                StorageLive(_) | StorageDead(_) => {}
                // Defined as a nop at runtime
                FakeRead(_)
                | Retag(..)
                | PlaceMention(_)
                | AscribeUserType(..)
                | ConstEvalCounter
                | Nop
                | BackwardIncompatibleDropHint { .. } => {}
                // This is a nop rn, but could be useful later
                Coverage(_) => {}
                Assign(box (place, rval)) => {
                    writeln!(asm, "\t{COMMENT_CHAR} TODO: assign {rval:?} to {place:?}").unwrap();
                    writeln!(asm, "{}", get_assignment_asm(tcx, &locals, place, rval)).unwrap();
                }

                _ => asm += format!("\t{COMMENT_CHAR} TODO: {:?}\n", statement.kind).as_str(),
            }
        }
        // generate terminator
        match &bb.terminator().kind {
            TerminatorKind::Goto { target } => {
                asm += format!("\tj {}\n", j_addr_for(id, *target)).as_str()
            }
            TerminatorKind::Return => writeln!(asm, "\tj {END_BB_IDX}f").unwrap(),
            TerminatorKind::SwitchInt { discr: Operand::Copy(place), targets }
            | TerminatorKind::SwitchInt { discr: Operand::Move(place), targets } => {
                asm += format!("\t{COMMENT_CHAR} branching on {:?} {:?}\n", place, place.projection).as_str();

                for (discr, target) in targets.iter() {
                    asm += format!("\t{COMMENT_CHAR} {discr} -> {target:?}\n").as_str();
                    asm += format!("\tli\tt0, {discr}\n").as_str();
                    // TODO properly handle discriminant size
                    // TODO properly handle place finding
                    asm += format!("\tld\tt1, {}(sp)\n", stack_offset_for(&locals, place.local))
                        .as_str();
                    asm += format!("\tbeq\tt0, t1, {}\n", j_addr_for(id, target)).as_str();
                }
                let otherwise_idx = usize::from(targets.otherwise());
                asm += format!(
                    "\t{}j {}\n",
                    // add comment to show that we want to jump to next block
                    // but ommit actual instruction so w just fall through
                    // yay optimizations
                    if otherwise_idx == id + 1 { &COMMENT_CHAR } else { "" },
                    j_addr_for(id, targets.otherwise())
                )
                .as_str();
            }
            _ => asm += format!("\t{COMMENT_CHAR} TODO: {:?}\n\n", bb.terminator().kind).as_str(),
        }
    }

    writeln!(asm, "{END_BB_IDX}:").unwrap();
    asm += format!("\taddi\tsp, sp, {}\n", -stack_offset).as_str();
    asm += "\tret\n\n";
    return asm;
}

/// Return the size on the stack of ty in bytes
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
        // TyKind::Array(t, n) => local_size(t.kind(), tcx).map(|s| {
        //     if let ConstKind::Unevaluated(c) = n.kind() {
        //         return tcx.const_eval_resolve(TypingEnv::fully_monomorphized(), c, DUMMY_SP)
        //             .expect(format!("Array len not resolvable to usize! type {:?}", ty).as_str())
        //             .try_to_target_usize(tcx)
        //             .expect(format!("Array len not convertible to usize! type {:?}", ty).as_str())
        //             as usize * s;
        //     };
        //     s * n
        //         .try_to_target_usize(tcx)
        //         .expect(format!("Array len not convertible to usize! type {:?}", ty).as_str())
        //         as usize
        // }),
        TyKind::Tuple(tys) => tys.iter().map(|t| local_size(t.kind(), tcx)).sum(),
        // adts
        TyKind::Adt(adt, gargs) => adt_size(tcx, *adt, gargs),
        // zst
        TyKind::Never => Ok(0),

        _ => Err(format!("unknown type: {:?} kind: {:?}", ty, ty)),
    }
}

#[test]
pub fn test_type_sizes() {
    let tcx = (); // TODO fix ??????
    assert_equal!(aot::local_size(TyKind::Bool), Ok(1));

    assert_equal!(aot::local_size(TyKind::Char), Ok(1));

    assert_equal!(aot::local_size(TyKind::Int(IntTy::I32)), Ok(4));

    assert_equal!(aot::local_size(TyKind::Uint(UintTy::U32)), Ok(4));

    assert_equal!(aot::local_size(TyKind::Float(FloatTy::F32)), Ok(4));
}

fn adt_size<'tcx>(
    tcx: TyCtxt<'tcx>,
    adt: AdtDef<'tcx>,
    gargs: &'tcx GenericArgs<'tcx>,
) -> Result<usize, String> {
    if adt.variants().len() == 0 {
        return Ok(0);
    };
    adt.variants()
        .into_iter()
        .map(|v| {
            v.fields
                .iter()
                .map(|f| match local_size(f.ty(tcx, gargs).kind(), tcx) {
                    Ok(n) => n,
                    // TODO handle errors properly
                    Err(_err) => {
                        // eprintln!("error getting size for {:?} error was {}", v.fields, err);
                        0
                    }
                })
                .sum()
        })
        .max()
        .ok_or(format!("Max failed for {:?} {:?}", adt, gargs))
}

fn get_assignment_asm<'tcx>(
    tcx: TyCtxt<'tcx>,
    locals: &Vec<Local<'_>>,
    place: &Place<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> String {
    let mut asm = String::new();

    // Calculate result
    use rustc_middle::mir::Rvalue::*;
    match rvalue {
        Use(operand) => {
            writeln!(asm, "\t{COMMENT_CHAR} Place {place:?}{:?} = Use({operand:?})", place.projection).unwrap();
            writeln!(asm, "{}", load_operand(tcx, locals, operand, "t0")).unwrap();
            writeln!(asm, "{}", reg_to_place(tcx, locals, place, "t0")).unwrap();
        },
        UnaryOp(operation, operand) => {
            use rustc_middle::mir::UnOp::*;
            writeln!(asm, "\t{COMMENT_CHAR} Place {place:?}{:?} = {operation:?}({operand:?})", place.projection).unwrap();
            match operation {
                Not => {
                    writeln!(asm, "{}", load_operand(tcx, &locals, operand, "t0")).unwrap();
                    writeln!(asm, "\tnot\tt0, t0").unwrap();
                    writeln!(asm, "{}", reg_to_place(tcx, &locals, place, "t0")).unwrap();
                }
                Neg => {
                    writeln!(asm, "{}", load_operand(tcx, &locals, operand, "t0")).unwrap();
                    writeln!(asm, "\tneg\tt0, t0").unwrap();
                    writeln!(asm, "{}", reg_to_place(tcx, &locals, place, "t0")).unwrap();
                }
                PtrMetadata => {
                    writeln!(asm, "\t{COMMENT_CHAR} TODO Metadata").unwrap();
                }
                
            }
        },

        _ => {
            return format!(
                "\t{COMMENT_CHAR} unknown rvalue {rvalue:?} when trying to assign to {place:?}"
            );
        }
    }
    // TODO handle non local Places
    if place.projection.len() > 0 {
        return format!(
            "\t{COMMENT_CHAR} place has projections {place:?} when trying to assign {rvalue:?} to it"
        );
    }

    return asm;
}

// fn get_operand_ty<'tcx>(
//     tcx: TyCtxt<'tcx>,
//     locals: &Vec<Local<'_>>,
//     op: &Operand<'tcx>,
// ) -> Ty<'tcx> {
//     use rustc_middle::mir::Operand::*;
//     match op {
//         Move(place) | Copy(place) => place_to_reg(tcx, locals, &place, dest),
//         Constant(box ConstOperand { const_: con, span, .. }) => {
//             use rustc_middle::mir::ConstValue::*;
//             use rustc_middle::mir::interpret::Scalar::*;
//             let evaluated_con = con.eval(tcx, TypingEnv::fully_monomorphized(), *span).unwrap();
//             match evaluated_con {
//                 Scalar(Int(scalar_int)) => {
//                     return format!(
//                         "\t{COMMENT_CHAR} loading {evaluated_con:?}\n\tli\t{dest}, 0x{:x}\n",
//                         scalar_int.to_bits_unchecked()
//                     );
//                 }
//                 _ => {}
//             }
//             return format!("\t{COMMENT_CHAR} loading {evaluated_con:?}\n");
//         }
//     }   
// }

fn load_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    locals: &Vec<Local<'_>>,
    op: &Operand<'tcx>,
    dest: &str,
) -> String {
    use rustc_middle::mir::Operand::*;
    match op {
        Move(place) | Copy(place) => place_to_reg(tcx, locals, &place, dest),
        Constant(box ConstOperand { const_: con, span, .. }) => {
            use rustc_middle::mir::ConstValue::*;
            use rustc_middle::mir::interpret::Scalar::*;
            let evaluated_con = con.eval(tcx, TypingEnv::fully_monomorphized(), *span).unwrap();
            match evaluated_con {
                Scalar(Int(scalar_int)) => {
                    return format!(
                        "\t{COMMENT_CHAR} loading {evaluated_con:?}\n\tli\t{dest}, 0x{:x}\n",
                        scalar_int.to_bits_unchecked()
                    );
                }
                _ => {}
            }
            return format!("\t{COMMENT_CHAR} loading {evaluated_con:?}\n");
        }
    }
}

fn place_to_reg<'tcx>(
    tcx: TyCtxt<'tcx>,
    locals: &Vec<Local<'_>>,
    place: &Place<'tcx>,
    dest: &str,
) -> String {
    place_to_reg_recursive(tcx, locals, place, dest, 0)
}

// TODO make amore general place traversal algo
fn place_to_reg_recursive<'tcx>(
    tcx: TyCtxt<'tcx>,
    locals: &Vec<Local<'_>>,
    place: &Place<'tcx>,
    dest: &str,
    projection_idx: usize,
) -> String {
    if projection_idx == 0 {
        // TODO load right number of bytes
        return format!(
            "\tld t0, {}(sp)\n{}",
            stack_offset_for(locals, place.local),
            place_to_reg_recursive(tcx, locals, place, dest, 1)
        );
    }
    if projection_idx >= place.projection.len() {
        return String::new();
    }
    use rustc_middle::mir::ProjectionElem::*;
    return match &place.projection[projection_idx] {
        Deref => {
            // TODO load right number of bytes
            format!(
                "\tld t0, 0(t0)\n{}",
                place_to_reg_recursive(tcx, locals, place, dest, projection_idx + 1)
            )
        }
        _ => format!(
            "\tli t0, 0 {COMMENT_CHAR} unknown projection {:?}",
            place.projection[projection_idx]
        ),
    };
}

fn reg_to_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    locals: &Vec<Local<'_>>,
    place: &Place<'tcx>,
    src: &str,
) -> String {
    if place.projection.len() == 0 {
        return format!("\tsd {}, {}(sp)\n", src, stack_offset_for(locals, place.local));
    }
    reg_to_place_recursive(tcx, locals, place, src, 0)
}

fn reg_to_place_recursive<'tcx>(
    tcx: TyCtxt<'tcx>,
    locals: &Vec<Local<'_>>,
    place: &Place<'tcx>,
    src: &str,
    projection_idx: usize,
) -> String {
    let scratch_reg = if src == "t0" { "t1" } else { "t0" };
    if projection_idx == 0 {
        // TODO load right number of bytes
        return format!(
            "\tld {scratch_reg}, {}(sp)\n{}",
            stack_offset_for(locals, place.local),
            reg_to_place_recursive(tcx, locals, place, src, 1)
        );
    }
    if projection_idx >= place.projection.len() {
        // TODO store correct number of bytes
        return format!("\tsd\t{src}, 0({scratch_reg})\n");
    }
    use rustc_middle::mir::ProjectionElem::*;
    return match &place.projection[projection_idx] {
        Deref => {
            // TODO load right number of bytes
            format!(
                "\tld {scratch_reg}, 0(t0)\n{}",
                reg_to_place_recursive(tcx, locals, place, src, projection_idx + 1)
            )
        }
        _ => format!(
            "\tli {scratch_reg}, 0 {COMMENT_CHAR} unknown projection {:?}\n",
            place.projection[projection_idx]
        ),
    };
}

fn stack_offset_for(locals: &Vec<Local<'_>>, target: rustc_middle::mir::Local) -> usize {
    locals[usize::from(target)..].iter().map(|l| l.stack_size.as_ref().unwrap_or(&0)).sum::<usize>()
}
