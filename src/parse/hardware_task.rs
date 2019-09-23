use syn::{parse, ItemFn};

use crate::{
    ast::{HardwareTask, HardwareTaskArgs, Local},
    parse::util,
};

impl HardwareTask {
    pub(crate) fn parse(args: HardwareTaskArgs, item: ItemFn, cores: u8) -> parse::Result<Self> {
        let span = item.sig.ident.span();
        let valid_signature = util::check_fn_signature(&item)
            && item.sig.inputs.len() == 1
            && (util::type_is_unit(&item.sig.output) || util::type_is_generator(&item.sig.output));

        let name = item.sig.ident.to_string();

        if name == "init" || name == "idle" {
            return Err(parse::Error::new(
                span,
                "tasks cannot be named `init` or `idle`",
            ));
        }

        if valid_signature {
            if let Some((context, Ok(rest))) = util::parse_inputs(item.sig.inputs, &name) {
                if rest.is_empty() {
                    let (locals, stmts) = util::extract_locals(item.block.stmts)?;
                    let attrs = item.attrs;
                    let is_generator = util::type_is_generator(&item.sig.output);

                    return Ok(HardwareTask {
                        args,
                        is_generator,
                        attrs,
                        context,
                        locals: Local::parse(locals, cores)?,
                        stmts,
                        _extensible: (),
                    });
                }
            }
        }

        Err(parse::Error::new(
            span,
            &format!(
                "this task handler must have type signature `fn({}::Context)`",
                name
            ),
        ))
    }
}
