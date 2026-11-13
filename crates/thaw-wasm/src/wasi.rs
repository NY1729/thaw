//! WASI preview1 on top of `wasmi_wasi` (wasi-common, cap-std sandboxed dirs).
//! Stdio is inherited from the process, as in the QuickJS wasm host.

use std::path::PathBuf;

use crate::{
    Access, HostCallbacks, Import, InstanceId, InstantiateError, LinkError, Module, Store,
};
use wasmi_wasi::sync::{ambient_authority, Dir, WasiCtxBuilder};

/// What `wasi_snapshot_preview1` sees. `preopens` maps a guest path to a host directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WasiOptions {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub preopens: Vec<(String, PathBuf)>,
}

pub(crate) fn add_to_linker(
    linker: &mut wasmi::Linker<crate::HostState>,
) -> Result<(), wasmi_wasi::Error> {
    wasmi_wasi::add_to_linker(linker, |state: &mut crate::HostState| &mut state.wasi)
}

fn build_ctx(options: &WasiOptions) -> Result<wasmi_wasi::WasiCtx, String> {
    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    builder.args(&options.args).map_err(|e| e.to_string())?;
    builder.envs(&options.env).map_err(|e| e.to_string())?;
    for (guest, host) in &options.preopens {
        let dir = Dir::open_ambient_dir(host, ambient_authority()).map_err(|e| e.to_string())?;
        builder
            .preopened_dir(dir, guest)
            .map_err(|e| e.to_string())?;
    }
    Ok(builder.build())
}

impl Access<'_, '_> {
    /// Instantiates with `wasi_snapshot_preview1` imports resolved from `options`.
    /// One WASI context per store: a later call replaces the context of earlier instances.
    /// ponytail: per-instance context needs one store per WASI instance; add when thaw-js needs it.
    pub fn instantiate_with_wasi(
        &mut self,
        module: &Module,
        imports: &[Import],
        host: &mut dyn HostCallbacks,
        options: &WasiOptions,
    ) -> Result<InstanceId, InstantiateError> {
        let ctx = build_ctx(options).map_err(|message| {
            InstantiateError::Link(LinkError {
                type_error: false,
                message,
            })
        })?;
        self.ctx.data_mut().wasi = ctx;
        self.instantiate_inner(module, imports, host, true)
    }
}

impl Store {
    /// See [`Access::instantiate_with_wasi`].
    pub fn instantiate_with_wasi(
        &mut self,
        module: &Module,
        imports: &[Import],
        host: &mut dyn HostCallbacks,
        options: &WasiOptions,
    ) -> Result<InstanceId, InstantiateError> {
        self.access()
            .instantiate_with_wasi(module, imports, host, options)
    }
}
