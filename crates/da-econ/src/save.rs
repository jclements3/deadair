//! Versioned RON save files for the whole business state (SDD §10:
//! "Save data: versioned RON (money, rep, upgrades, contract state)").

use crate::business::Business;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Current save format version.
pub const SAVE_VERSION: u32 = 1;

/// On-disk save envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveFile {
    /// Format version; loads fail on mismatch rather than misread.
    pub version: u32,
    /// The whole business state.
    pub business: Business,
}

/// Errors from save/load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// RON (de)serialization failed.
    Ron(String),
    /// The file's version is not [`SAVE_VERSION`].
    VersionMismatch {
        /// Version found in the file.
        found: u32,
        /// Version this build reads.
        expected: u32,
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Ron(msg) => write!(f, "RON error: {msg}"),
            SaveError::VersionMismatch { found, expected } => {
                write!(f, "save version {found}, expected {expected}")
            }
        }
    }
}

impl std::error::Error for SaveError {}

/// Serialize the business to a versioned RON string.
pub fn save_to_ron(business: &Business) -> Result<String, SaveError> {
    let file = SaveFile {
        version: SAVE_VERSION,
        business: business.clone(),
    };
    ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
        .map_err(|e| SaveError::Ron(e.to_string()))
}

/// Load a business from a versioned RON string, rejecting foreign versions.
pub fn load_from_ron(text: &str) -> Result<Business, SaveError> {
    let file: SaveFile = ron::from_str(text).map_err(|e| SaveError::Ron(e.to_string()))?;
    if file.version != SAVE_VERSION {
        return Err(SaveError::VersionMismatch {
            found: file.version,
            expected: SAVE_VERSION,
        });
    }
    Ok(file.business)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_mismatch_is_rejected() {
        let mut text = save_to_ron(&Business::new()).unwrap();
        text = text.replace("version: 1", "version: 99");
        assert!(matches!(
            load_from_ron(&text),
            Err(SaveError::VersionMismatch { found: 99, .. })
        ));
    }
}
