pub mod adapter;
pub mod browser_integration;
pub mod download_job;
pub mod download_settings;
pub mod messaging;
pub mod timestamp;

pub const SCHEMA_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    use super::SCHEMA_VERSION;

    #[test]
    fn the_typescript_contracts_name_the_same_schema_version() {
        let contracts = include_str!("../../../packages/contracts/src/index.ts");

        assert!(
            contracts.contains(&format!(
                "export const SCHEMA_VERSION = {SCHEMA_VERSION} as const;"
            )),
            "SCHEMA_VERSION in packages/contracts/src/index.ts must be {SCHEMA_VERSION}"
        );
    }
}
