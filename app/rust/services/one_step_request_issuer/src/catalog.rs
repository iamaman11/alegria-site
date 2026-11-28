//! The installed owner catalog is operator-controlled private configuration,
//! never supplied by a request locator or by a caller-authored DAG.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use operation_protocol::{OperationContract, PinnedOwnerRouteV1, TrustedRouteCatalog};
use serde::{Deserialize, Serialize};

use crate::private_file;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledOwnerCatalogV1 {
    contracts: Vec<OperationContract>,
    routes: Vec<PinnedOwnerRouteV1>,
}

pub(crate) fn read_installed_catalog(path: &Path) -> Result<TrustedRouteCatalog> {
    let bytes = private_file::read_private_file(path)?;
    let installed: InstalledOwnerCatalogV1 =
        serde_json::from_slice(&bytes).context("decode installed independent owner catalog")?;
    ensure!(
        serde_json::to_vec(&installed)? == bytes,
        "installed owner catalog must be exact canonical JSON"
    );
    let mut routes = installed.routes;
    routes.sort_by(|left, right| left.operation.cmp(&right.operation));
    ensure!(
        routes
            .windows(2)
            .all(|pair| pair[0].operation != pair[1].operation),
        "duplicate operation in installed owner catalog"
    );
    let mut catalog = TrustedRouteCatalog::default();
    for contract in installed.contracts {
        let index = routes
            .iter()
            .position(|route| route.operation == contract.reference)
            .context("installed owner contract has no matching route")?;
        catalog
            .register(contract, routes.remove(index))
            .context("invalid installed operation owner contract/route")?;
    }
    ensure!(routes.is_empty(), "installed owner route lacks contract");
    Ok(catalog)
}
