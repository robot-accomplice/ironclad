use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IroncladError, Result};

include!("config/preamble_types.rs");

const BUNDLED_PROVIDERS_TOML: &str = include_str!("bundled_providers.toml");

#[derive(Debug, Clone, Deserialize, Default)]
struct BundledProviders {
    #[serde(default)]
    providers: HashMap<String, ProviderConfig>,
}

include!("config/impl_core.rs");

include!("config/migration.rs");

include!("config/agent_paths.rs");
include!("config/model_wallet.rs");
include!("config/runtime_sections.rs");

#[cfg(test)]
mod tests;
