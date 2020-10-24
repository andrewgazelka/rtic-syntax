//! Abstract Syntax Tree

use core::ops::Deref;

use syn::{Attribute, Expr, Ident, Item, ItemUse, Pat, PatType, Path, Stmt, Type};

use crate::{Map, Set};

/// The `#[app]` attribute
#[derive(Debug)]
pub struct App {
    /// The arguments to the `#[app]` attribute
    pub args: AppArgs,

    /// The name of the `const` item on which the `#[app]` attribute has been placed
    pub name: Ident,

    /// Vector containing the `#[init]` function
    pub inits: Inits,

    /// Vector containing the `#[idle]` function
    pub idles: Idles,

    /// Late (runtime initialized) resources
    pub late_resources: Map<LateResource>,

    /// Early (compile time initialized) resources
    pub resources: Map<Resource>,

    /// User imports
    pub user_imports: Vec<ItemUse>,

    /// User code
    pub user_code: Vec<Item>,

    /// Hardware tasks: `#[task(binds = ..)]`s
    pub hardware_tasks: Map<HardwareTask>,

    /// Software tasks: `#[task]`
    pub software_tasks: Map<SoftwareTask>,

    pub(crate) _extensible: (),
}

/// Interrupts used to dispatch software tasks
pub type ExternInterrupts = Map<ExternInterrupt>;

/// Interrupt that could be used to dispatch software tasks
#[derive(Debug, Clone)]
pub struct ExternInterrupt {
    /// Attributes that will apply to this interrupt handler
    pub attrs: Vec<Attribute>,

    pub(crate) _extensible: (),
}

/// The arguments of the `#[app]` attribute
#[derive(Debug)]
pub struct AppArgs {
    /// Device
    pub device: Option<Path>,

    /// Monotonic
    pub monotonic: Option<Path>,

    /// Peripherals
    pub peripherals: bool,

    /// Interrupts used to dispatch software tasks
    pub extern_interrupts: ExternInterrupts,
}

/// `init` function
pub type Inits = Vec<Init>;

/// `idle` function
pub type Idles = Vec<Idle>;

/// The `init`-ialization function
#[derive(Debug)]
pub struct Init {
    /// `init` context metadata
    pub args: InitArgs,

    /// Attributes that will apply to this `init` function
    pub attrs: Vec<Attribute>,

    /// The name of the `#[init]` function
    pub name: Ident,

    /// The context argument
    pub context: Box<Pat>,

    /// Static variables local to this context
    pub locals: Map<Local>,

    /// The statements that make up this `init` function
    pub stmts: Vec<Stmt>,

    pub(crate) _extensible: (),
}

/// `init` context metadata
#[derive(Debug, Default)]
pub struct InitArgs {
    /// Late resources that will be initialized
    ///
    /// NOTE do not use this field for codegen; use `Analysis.late_resources` instead
    pub late: Set<Ident>,

    /// Resources that can be accessed from this context
    pub resources: Resources,

    pub(crate) _extensible: (),
}

/// The `idle` context
#[derive(Debug)]
pub struct Idle {
    /// `idle` context metadata
    pub args: IdleArgs,

    /// Attributes that will apply to this `idle` function
    pub attrs: Vec<Attribute>,

    /// The name of the `#[idle]` function
    pub name: Ident,

    /// The context argument
    pub context: Box<Pat>,

    /// Static variables local to this context
    pub locals: Map<Local>,

    /// The statements that make up this `idle` function
    pub stmts: Vec<Stmt>,

    pub(crate) _extensible: (),
}

/// `idle` context metadata
#[derive(Debug)]
pub struct IdleArgs {
    /// Resources that can be accessed from this context
    pub resources: Resources,

    pub(crate) _extensible: (),
}

/// Resource properties
#[derive(Debug)]
pub struct ResourceProperties {
    /// A task local resource
    pub task_local: bool,

    /// A lock free (exclusive resource)
    pub lock_free: bool,
}

/// An early (compile time initialized) resource
#[derive(Debug)]
pub struct Resource {
    pub(crate) late: LateResource,
    /// The initial value of this resource
    pub expr: Box<Expr>,
}

impl Deref for Resource {
    type Target = LateResource;

    fn deref(&self) -> &LateResource {
        &self.late
    }
}

/// A late (runtime initialized) resource
#[derive(Debug)]
pub struct LateResource {
    /// `#[cfg]` attributes like `#[cfg(debug_assertions)]`
    pub cfgs: Vec<Attribute>,

    /// Attributes that will apply to this resource
    pub attrs: Vec<Attribute>,

    /// The type of this resource
    pub ty: Box<Type>,

    /// Resource properties
    pub properties: ResourceProperties,

    pub(crate) _extensible: (),
}

/// A software task
#[derive(Debug)]
pub struct SoftwareTask {
    /// Software task metadata
    pub args: SoftwareTaskArgs,

    /// `#[cfg]` attributes like `#[cfg(debug_assertions)]`
    pub cfgs: Vec<Attribute>,

    /// Attributes that will apply to this interrupt handler
    pub attrs: Vec<Attribute>,

    /// The context argument
    pub context: Box<Pat>,

    /// The inputs of this software task
    pub inputs: Vec<PatType>,

    /// Static variables local to this context
    pub locals: Map<Local>,

    /// The statements that make up the task handler
    pub stmts: Vec<Stmt>,

    /// The task is declared externally
    pub is_extern: bool,

    pub(crate) _extensible: (),
}

/// Software task metadata
#[derive(Debug)]
pub struct SoftwareTaskArgs {
    /// The task capacity: the maximum number of pending messages that can be queued
    pub capacity: u8,

    /// The priority of this task
    pub priority: u8,

    /// Resources that can be accessed from this context
    pub resources: Resources,

    pub(crate) _extensible: (),
}

impl Default for SoftwareTaskArgs {
    fn default() -> Self {
        Self {
            capacity: 1,
            priority: 1,
            resources: Resources::new(),
            _extensible: (),
        }
    }
}

/// A hardware task
#[derive(Debug)]
pub struct HardwareTask {
    /// Hardware task metadata
    pub args: HardwareTaskArgs,

    /// Attributes that will apply to this interrupt handler
    pub attrs: Vec<Attribute>,

    /// The context argument
    pub context: Box<Pat>,

    /// Static variables local to this context
    pub locals: Map<Local>,

    /// The statements that make up the task handler
    pub stmts: Vec<Stmt>,

    /// The task is declared externally
    pub is_extern: bool,

    pub(crate) _extensible: (),
}

/// Hardware task metadata
#[derive(Debug)]
pub struct HardwareTaskArgs {
    /// The interrupt or exception that this task is bound to
    pub binds: Ident,

    /// The priority of this task
    pub priority: u8,

    /// Resources that can be accessed from this context
    pub resources: Resources,

    pub(crate) _extensible: (),
}

/// A `static mut` variable local to and owned by a context
#[derive(Debug)]
pub struct Local {
    /// Attributes like `#[link_section]`
    pub attrs: Vec<Attribute>,

    /// `#[cfg]` attributes like `#[cfg(debug_assertions)]`
    pub cfgs: Vec<Attribute>,

    /// Type
    pub ty: Box<Type>,

    /// Initial value
    pub expr: Box<Expr>,

    pub(crate) _extensible: (),
}

/// Resource access
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Access {
    /// `[x]`, a mutable resource
    Exclusive,

    /// `[&x]`, a static non-mutable resource
    Shared,
}

impl Access {
    /// Is this enum in the `Exclusive` variant?
    pub fn is_exclusive(&self) -> bool {
        *self == Access::Exclusive
    }

    /// Is this enum in the `Shared` variant?
    pub fn is_shared(&self) -> bool {
        *self == Access::Shared
    }
}

/// Resource access list
pub type Resources = Map<Access>;
