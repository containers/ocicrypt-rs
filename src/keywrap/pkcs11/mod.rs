// Copyright The ocicrypt Authors.
// SPDX-License-Identifier: Apache-2.0

use crate::config::{parse_pkcs11_config_file, DecryptConfig, EncryptConfig, Pkcs11Config};
use crate::keywrap::KeyWrapper;
use crate::pkcs11_uri_wrapped::Pkcs11UriWrapped;
use crate::utils::pkcs11::{
    decrypt_pkcs11, encrypt_multiple, parse_private_key, parse_public_key, Pkcs11KeyType,
};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

#[derive(Debug)]
pub struct Pkcs11KeyWrapper {}

// Pkcs11KeyFileObject is a representation of the Pkcs11KeyFile with the pkcs11
// URI wrapper as an object
pub struct Pkcs11KeyFileObject {
    pub uriw: Pkcs11UriWrapped,
}

impl KeyWrapper for Pkcs11KeyWrapper {
    // Wrap the session key for recpients and encrypt the opts_data,
    // which describe the symmetric key used for encrypting the layer
    fn wrap_keys(&self, ec: &EncryptConfig, opts_data: &[u8]) -> Result<Vec<u8>> {
        let mut pubkeys: Vec<Vec<u8>> = Vec::new();
        if let Some(pks) = ec.param.get("pkcs11-pubkeys") {
            pubkeys.extend(pks.clone());
        }
        if let Some(yamls) = ec.param.get("pkcs11-yamls") {
            pubkeys.extend(yamls.clone());
        };
        let decrypt_config_pubkeys = match ec.decrypt_config.as_ref() {
            Some(x) => x,
            None => {
                return Err(anyhow!(
                    "EncryptConfig is missing
                                        decrypt_config member"
                ))
            }
        };

        let pkcs11_recipients: Vec<Pkcs11KeyType> = add_pub_keys(decrypt_config_pubkeys, &pubkeys)?;

        if pkcs11_recipients.is_empty() {
            return Ok(Vec::new());
        }

        encrypt_multiple(&pkcs11_recipients, opts_data)
    }

    fn unwrap_keys(&self, dc: &DecryptConfig, annotation: &[u8]) -> Result<Vec<u8>> {
        let priv_keys: Vec<Vec<u8>> = self
            .private_keys(&dc.param)
            .ok_or_else(|| anyhow!("No private keys found for PKCS11 decryption"))?;

        let p11conf_opt = p11conf_from_params(&dc.param)?;

        // Parse the private keys.
        // Then filter for just the "PKFO" (Pkcs11KeyFileObject) variants,
        // and update the module dirs and allowed modules paths if appropriate
        let pkcs11_keys: Vec<Box<Pkcs11KeyFileObject>> = priv_keys
            .iter()
            .map(|key| parse_private_key(key, &[], "PKCS11".to_string()))
            .collect::<Result<Vec<Pkcs11KeyType>>>()?
            .into_iter()
            .filter_map(|key| {
                if let Pkcs11KeyType::PKFO(mut p) = key {
                    if let Some(ref p11conf) = p11conf_opt {
                        p.uriw.set_module_directories(&p11conf.module_directories);
                        p.uriw
                            .set_allowed_module_paths(&p11conf.allowed_module_paths);
                    }
                    return Some(p);
                }
                None
            })
            .collect();
        decrypt_pkcs11(&pkcs11_keys, annotation)
    }

    fn annotation_id(&self) -> String {
        "org.opencontainers.image.enc.keys.pkcs11".to_string()
    }

    fn no_possible_keys(&self, dcparameters: &HashMap<String, Vec<Vec<u8>>>) -> bool {
        self.private_keys(dcparameters).is_none()
    }

    fn private_keys(&self, dcparameters: &HashMap<String, Vec<Vec<u8>>>) -> Option<Vec<Vec<u8>>> {
        dcparameters.get("pkcs11-yamls").cloned()
    }

    fn recipients(&self, _packet: String) -> Option<Vec<String>> {
        Some(vec!["[pkcs11]".to_string()])
    }
}

fn p11conf_from_params(
    dcparameters: &HashMap<String, Vec<Vec<u8>>>,
) -> Result<Option<Pkcs11Config>> {
    if dcparameters.contains_key("pkcs11-config") {
        return Ok(Some(parse_pkcs11_config_file(
            &dcparameters["pkcs11-config"][0],
        )?));
    }
    Ok(None)
}

fn add_pub_keys(dc: &DecryptConfig, pubkeys: &[Vec<u8>]) -> Result<Vec<Pkcs11KeyType>> {
    if pubkeys.is_empty() {
        return Ok(vec![]);
    }
    let p11conf_opt = p11conf_from_params(&dc.param)?;
    // parse and collect keys pkcs11 keys.
    // also update the module dirs and allowed module paths if appropriate
    let pkcs11_keys: Vec<Pkcs11KeyType> = pubkeys
        .iter()
        .map(|key| parse_public_key(key, "PKCS11".to_string()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|mut key| {
            if let Pkcs11KeyType::PKFO(ref mut p) = key {
                if let Some(ref p11conf) = p11conf_opt {
                    p.uriw.set_module_directories(&p11conf.module_directories);
                    p.uriw
                        .set_allowed_module_paths(&p11conf.allowed_module_paths);
                }
            }
            key
        })
        .collect();
    Ok(pkcs11_keys)
}

/*#[cfg(test)]
mod kw_tests {
    use super::*;
    use crate::config::{get_default_module_directories_yaml, CryptoConfig};
    use crate::softhsm::SoftHSMSetup;

    const SOFTHSM_SETUP: &str = "scripts/softhsm_setup";

    #[test]
    fn test_keywrap_pkcs11_success() {
        let (valid_pkcs11_ccs, shsm) = create_valid_pkcs11_ccs().unwrap();

        std::env::set_var("OCICRYPT_OAEP_HASHALG", "sha1");

        for cc in valid_pkcs11_ccs {
            let kw = Pkcs11KeyWrapper {};

            let data = "This is some secret text".as_bytes();

            if let Some(ec) = cc.encrypt_config {
                let wk = kw.wrap_keys(&ec, data).unwrap();
                if let Some(dc) = cc.decrypt_config {
                    let ud = kw.unwrap_keys(&dc, &wk).unwrap();
                    assert_eq!(data, ud);
                } else {
                    panic!();
                }
            } else {
                panic!();
            }
        }

        assert!(shsm
            .run_softhsm_teardown(&SOFTHSM_SETUP.to_string())
            .is_ok());
    }

    #[test]
    fn test_annotation_id() {
        let pkcs11_key_wrapper = Pkcs11KeyWrapper {};
        assert_eq!(
            pkcs11_key_wrapper.annotation_id(),
            "org.opencontainers.image.enc.keys.pkcs11"
        );
    }

    #[test]
    fn test_no_possible_keys() {
        let pkcs11_key_wrapper = Pkcs11KeyWrapper {};
        let dc = DecryptConfig::default();
        assert!(pkcs11_key_wrapper.no_possible_keys(&dc.param));
    }

    #[test]
    fn test_private_keys() {
        let pkcs11_key_wrapper = Pkcs11KeyWrapper {};
        let dc = DecryptConfig::default();
        assert!(pkcs11_key_wrapper.private_keys(&dc.param).is_none());
    }

    #[test]
    fn test_key_ids_from_packet() {
        let pkcs11_key_wrapper = Pkcs11KeyWrapper {};
        assert!(pkcs11_key_wrapper.keyids_from_packet("".to_string()) == None);
    }

    #[test]
    fn test_recipients() {
        let pkcs11_key_wrapper = Pkcs11KeyWrapper {};
        let recipients = pkcs11_key_wrapper.recipients("".to_string()).unwrap();
        assert!(recipients.len() == 1);
        assert!(recipients[0] == "[pkcs11]");
    }

    fn get_pkcs11_config_yaml() -> Result<Vec<u8>> {
        // we need to provide a configuration file so that on the various
        // distros the libsofthsm2.so will be found by searching directories
        let mdyaml = get_default_module_directories_yaml("".to_string())?;
        let config = format!(
            "module_directories:\n\
                              {}\
                              allowed_module_paths:\n\
                              {}",
            mdyaml, mdyaml
        );
        Ok(config.as_bytes().to_vec())
    }

    fn create_valid_pkcs11_ccs() -> Result<(Vec<CryptoConfig>, SoftHSMSetup)> {
        let shsm = SoftHSMSetup::new()?;
        let pkcs11_pubkey_uri_str = shsm.run_softhsm_setup(&SOFTHSM_SETUP.to_string())?;
        let pubkey_pem = shsm.run_softhsm_get_pubkey(&SOFTHSM_SETUP.to_string())?;
        let pkcs11_privkey_yaml = format!(
            "pkcs11:
  uri: {}
module:
  env:
    SOFTHSM2_CONF: {}",
            pkcs11_pubkey_uri_str,
            shsm.get_config_filename()?
        );
        let p11conf_yaml = get_pkcs11_config_yaml()?;

        let mut k1_ec_p = HashMap::new();
        k1_ec_p.insert(
            "pkcs11-pubkeys".to_string(),
            vec![pubkey_pem.as_bytes().to_vec()],
        );
        let mut k1_ec_dc_p = HashMap::new();
        k1_ec_dc_p.insert(
            "pkcs11-yamls".to_string(),
            vec![pkcs11_privkey_yaml.as_bytes().to_vec()],
        );
        k1_ec_dc_p.insert("pkcs11-config".to_string(), vec![p11conf_yaml.to_vec()]);
        let mut k1_dc_p = HashMap::new();
        k1_dc_p.insert(
            "pkcs11-yamls".to_string(),
            vec![pkcs11_privkey_yaml.as_bytes().to_vec()],
        );
        k1_dc_p.insert("pkcs11-config".to_string(), vec![p11conf_yaml.to_vec()]);

        let mut k2_ec_p = HashMap::new();
        // public and private key YAMLs are identical
        k2_ec_p.insert(
            "pkcs11-yamls".to_string(),
            vec![pkcs11_privkey_yaml.as_bytes().to_vec()],
        );
        let mut k2_ec_dc_p = HashMap::new();
        k2_ec_dc_p.insert(
            "pkcs11-yamls".to_string(),
            vec![pkcs11_privkey_yaml.as_bytes().to_vec()],
        );
        k2_ec_dc_p.insert("pkcs11-config".to_string(), vec![p11conf_yaml.to_vec()]);
        let mut k2_dc_p = HashMap::new();
        k2_dc_p.insert(
            "pkcs11-yamls".to_string(),
            vec![pkcs11_privkey_yaml.as_bytes().to_vec()],
        );
        k2_dc_p.insert("pkcs11-config".to_string(), vec![p11conf_yaml.to_vec()]);

        let valid_pkcs11_ccs: Vec<CryptoConfig> = vec![
            // Key 1
            CryptoConfig {
                encrypt_config: Some(EncryptConfig {
                    param: k1_ec_p,
                    decrypt_config: Some(DecryptConfig { param: k1_ec_dc_p }),
                }),
                decrypt_config: Some(DecryptConfig { param: k1_dc_p }),
            },
            // Key 2
            CryptoConfig {
                encrypt_config: Some(EncryptConfig {
                    param: k2_ec_p,
                    decrypt_config: Some(DecryptConfig { param: k2_ec_dc_p }),
                }),
                decrypt_config: Some(DecryptConfig { param: k2_dc_p }),
            },
        ];
        Ok((valid_pkcs11_ccs, shsm))
    }
}*/
