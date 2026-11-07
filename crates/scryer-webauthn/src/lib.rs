use std::io::Cursor;
use std::ops::Deref;

use aws_lc_rs::{digest, rand::SecureRandom, rand::SystemRandom, signature};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ciborium::value::Value as CborValue;
use coset::{AsCborValue, CborSerializable, CoseKey, CoseKeyBuilder, iana};
use passkey_types::{
    Bytes,
    webauthn::{
        AttestationConveyancePreference, AttestationStatementFormatIdentifiers,
        AuthenticatorSelectionCriteria, PublicKeyCredentialDescriptor, PublicKeyCredentialHints,
        PublicKeyCredentialParameters, PublicKeyCredentialRpEntity, PublicKeyCredentialType,
        PublicKeyCredentialUserEntity, ResidentKeyRequirement, UserVerificationRequirement,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use url::Url;
use uuid::Uuid;

const CHALLENGE_LEN: usize = 32;
const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_BE: u8 = 0x08;
const FLAG_BS: u8 = 0x10;
const FLAG_AT: u8 = 0x40;
const FLAG_ED: u8 = 0x80;

pub mod prelude {
    pub use crate::{
        DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyAuthentication,
        PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    };
}

pub type WebauthnResult<T> = Result<T, WebauthnError>;

#[derive(Debug, thiserror::Error)]
pub enum WebauthnError {
    #[error("invalid relying party configuration: {0}")]
    InvalidRelyingParty(String),
    #[error("random challenge generation failed")]
    Random,
    #[error("invalid client data")]
    InvalidClientData,
    #[error("invalid credential payload")]
    InvalidCredential,
    #[error("invalid attestation object")]
    InvalidAttestationObject,
    #[error("unsupported attestation format {0}")]
    UnsupportedAttestationFormat(String),
    #[error("invalid authenticator data")]
    InvalidAuthenticatorData,
    #[error("relying party id hash mismatch")]
    RpIdHashMismatch,
    #[error("user presence is required")]
    UserPresenceRequired,
    #[error("user verification is required")]
    UserVerificationRequired,
    #[error("unsupported public key algorithm")]
    UnsupportedPublicKey,
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("credential was not found")]
    CredentialNotFound,
    #[error("signature counter replay detected")]
    CounterReplay,
    #[error("missing discoverable user handle")]
    MissingUserHandle,
    #[error("credential backup eligibility changed")]
    CredentialBackupEligibilityInconsistent,
    #[error("credential backup state is not hardware-bound")]
    CredentialMayNotBeHardwareBound,
}

#[derive(Debug, Clone)]
pub struct WebauthnBuilder {
    rp_id: String,
    rp_origin: String,
    rp_name: String,
}

impl WebauthnBuilder {
    pub fn new(rp_id: &str, origin: &Url) -> WebauthnResult<Self> {
        let rp_id = rp_id.trim();
        if rp_id.is_empty() {
            return Err(WebauthnError::InvalidRelyingParty(
                "rp id is required".to_string(),
            ));
        }

        let origin_host = origin.host_str().ok_or_else(|| {
            WebauthnError::InvalidRelyingParty("origin host is required".to_string())
        })?;
        let scheme = origin.scheme();
        let insecure_localhost =
            scheme == "http" && matches!(origin_host, "localhost" | "127.0.0.1" | "::1");
        if scheme != "https" && !insecure_localhost {
            return Err(WebauthnError::InvalidRelyingParty(
                "origin must use https unless it is localhost".to_string(),
            ));
        }

        if origin_host != rp_id && !origin_host.ends_with(&format!(".{rp_id}")) {
            return Err(WebauthnError::InvalidRelyingParty(
                "origin host must match the rp id or a subdomain".to_string(),
            ));
        }

        Ok(Self {
            rp_id: rp_id.to_string(),
            rp_origin: origin.origin().ascii_serialization(),
            rp_name: rp_id.to_string(),
        })
    }

    pub fn rp_name(mut self, rp_name: &str) -> Self {
        let rp_name = rp_name.trim();
        if !rp_name.is_empty() {
            self.rp_name = rp_name.to_string();
        }
        self
    }

    pub fn build(self) -> WebauthnResult<Webauthn> {
        Ok(Webauthn {
            rp_id: self.rp_id,
            rp_origin: self.rp_origin,
            rp_name: self.rp_name,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Webauthn {
    rp_id: String,
    rp_origin: String,
    rp_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialCreationOptions {
    pub public_key: PublicKeyCredentialCreationOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyCredentialCreationOptions {
    pub rp: PublicKeyCredentialRpEntity,
    pub user: PublicKeyCredentialUserEntity,
    pub challenge: Bytes,
    pub pub_key_cred_params: Vec<PublicKeyCredentialParameters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_credentials: Option<Vec<PublicKeyCredentialDescriptor>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticator_selection: Option<AuthenticatorSelectionCriteria>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<PublicKeyCredentialHints>>,
    pub attestation: AttestationConveyancePreference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_formats: Option<Vec<AttestationStatementFormatIdentifiers>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<AuthenticationExtensionsClientInputs>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRequestOptions {
    pub public_key: PublicKeyCredentialRequestOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyCredentialRequestOptions {
    pub challenge: Bytes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_credentials: Option<Vec<PublicKeyCredentialDescriptor>>,
    pub user_verification: UserVerificationRequirement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<PublicKeyCredentialHints>>,
    pub attestation: AttestationConveyancePreference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_formats: Option<Vec<AttestationStatementFormatIdentifiers>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<AuthenticationExtensionsClientInputs>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationExtensionsClientInputs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cred_props: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cred_protect: Option<CredentialProtectionInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uvm: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialProtectionInput {
    pub credential_protection_policy: CredentialProtectionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_credential_protection_policy: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialProtectionPolicy {
    UserVerificationRequired,
}

impl Webauthn {
    pub fn start_passkey_registration(
        &self,
        user_uuid: Uuid,
        username: &str,
        display_name: &str,
        exclude_credentials: Option<Vec<Vec<u8>>>,
    ) -> WebauthnResult<(CredentialCreationOptions, PasskeyRegistration)> {
        let challenge = random_bytes(CHALLENGE_LEN)?;
        let user_handle = user_uuid.as_bytes().to_vec();
        let options = CredentialCreationOptions {
            public_key: PublicKeyCredentialCreationOptions {
                rp: PublicKeyCredentialRpEntity {
                    id: Some(self.rp_id.clone()),
                    name: self.rp_name.clone(),
                },
                user: PublicKeyCredentialUserEntity {
                    id: user_handle.clone().into(),
                    display_name: display_name.to_string(),
                    name: username.to_string(),
                },
                challenge: challenge.clone().into(),
                pub_key_cred_params: vec![PublicKeyCredentialParameters {
                    ty: PublicKeyCredentialType::PublicKey,
                    alg: iana::Algorithm::ES256,
                }],
                timeout: None,
                exclude_credentials: exclude_credentials.map(|credentials| {
                    credentials
                        .into_iter()
                        .map(public_key_credential_descriptor)
                        .collect()
                }),
                authenticator_selection: Some(AuthenticatorSelectionCriteria {
                    authenticator_attachment: None,
                    resident_key: Some(ResidentKeyRequirement::Required),
                    require_resident_key: true,
                    user_verification: UserVerificationRequirement::Required,
                }),
                hints: None,
                attestation: AttestationConveyancePreference::None,
                attestation_formats: None,
                extensions: Some(registration_extensions()),
            },
        };

        Ok((
            options,
            PasskeyRegistration {
                challenge,
                user_uuid,
                user_handle,
                rp_id: self.rp_id.clone(),
                rp_origin: self.rp_origin.clone(),
            },
        ))
    }

    pub fn finish_passkey_registration(
        &self,
        credential: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> WebauthnResult<Passkey> {
        if state.rp_id != self.rp_id || state.rp_origin != self.rp_origin {
            return Err(WebauthnError::InvalidClientData);
        }
        if credential.type_ != "public-key" {
            return Err(WebauthnError::InvalidCredential);
        }

        validate_client_data_json(
            credential.response.client_data_json.as_slice(),
            "webauthn.create",
            &state.challenge,
            &self.rp_origin,
        )?;
        let attestation =
            parse_attestation_object(credential.response.attestation_object.as_slice())?;
        if attestation.fmt != "none" {
            return Err(WebauthnError::UnsupportedAttestationFormat(attestation.fmt));
        }
        if !attestation.att_stmt_empty {
            return Err(WebauthnError::InvalidAttestationObject);
        }
        let auth_data = parse_authenticator_data(&attestation.auth_data, Some(&self.rp_id))?;
        let attested = auth_data
            .attested_credential
            .ok_or(WebauthnError::InvalidAuthenticatorData)?;
        if credential.raw_id.as_slice() != attested.credential_id.as_slice() {
            return Err(WebauthnError::InvalidCredential);
        }
        validate_es256_cose_key(&attested.public_key_cose)?;

        Ok(Passkey {
            version: 1,
            cred_id: attested.credential_id,
            public_key_cose: attested.public_key_cose,
            user_handle: state.user_handle.clone(),
            counter: auth_data.counter,
            transports: credential.response.transports.clone(),
            user_verified: auth_data.user_verified,
            backup_eligible: auth_data.backup_eligible,
            backup_state: auth_data.backup_state,
        })
    }

    pub fn start_passkey_authentication(
        &self,
        passkeys: &[Passkey],
    ) -> WebauthnResult<(CredentialRequestOptions, PasskeyAuthentication)> {
        if passkeys.is_empty() {
            return Err(WebauthnError::CredentialNotFound);
        }
        let challenge = random_bytes(CHALLENGE_LEN)?;
        let options = self.request_options(
            challenge.clone(),
            Some(
                passkeys
                    .iter()
                    .map(|passkey| passkey.cred_id.clone())
                    .collect(),
            ),
        );

        Ok((
            options,
            PasskeyAuthentication {
                challenge,
                rp_id: self.rp_id.clone(),
                rp_origin: self.rp_origin.clone(),
                allowed_credentials: passkeys
                    .iter()
                    .map(|passkey| passkey.cred_id.clone())
                    .collect(),
            },
        ))
    }

    pub fn start_discoverable_authentication(
        &self,
    ) -> WebauthnResult<(CredentialRequestOptions, DiscoverableAuthentication)> {
        let challenge = random_bytes(CHALLENGE_LEN)?;
        let options = self.request_options(challenge.clone(), None);

        Ok((
            options,
            DiscoverableAuthentication {
                challenge,
                rp_id: self.rp_id.clone(),
                rp_origin: self.rp_origin.clone(),
            },
        ))
    }

    pub fn identify_discoverable_authentication<'a>(
        &self,
        credential: &'a PublicKeyCredential,
    ) -> WebauthnResult<(Uuid, &'a [u8])> {
        if credential.type_ != "public-key" {
            return Err(WebauthnError::InvalidCredential);
        }
        let user_handle = credential
            .response
            .user_handle
            .as_ref()
            .ok_or(WebauthnError::MissingUserHandle)?;
        let user_uuid = Uuid::from_slice(user_handle.as_slice())
            .map_err(|_| WebauthnError::MissingUserHandle)?;
        Ok((user_uuid, credential.raw_id.as_slice()))
    }

    pub fn finish_passkey_authentication(
        &self,
        credential: &PublicKeyCredential,
        state: &PasskeyAuthentication,
        passkey: &Passkey,
    ) -> WebauthnResult<AuthenticationResult> {
        if state.rp_id != self.rp_id || state.rp_origin != self.rp_origin {
            return Err(WebauthnError::InvalidClientData);
        }
        if !state
            .allowed_credentials
            .iter()
            .any(|credential_id| credential_id == credential.raw_id.as_slice())
        {
            return Err(WebauthnError::CredentialNotFound);
        }
        if passkey.cred_id != credential.raw_id.as_slice() {
            return Err(WebauthnError::CredentialNotFound);
        }
        if let Some(user_handle) = credential.response.user_handle.as_ref()
            && user_handle.as_slice() != passkey.user_handle.as_slice()
        {
            return Err(WebauthnError::InvalidCredential);
        }
        finish_authentication_with_passkey(
            credential,
            passkey,
            &state.challenge,
            self,
            BackupEligibilityPolicy::AllowFalseToTrueBackupEligibilityUpgrade,
        )
    }

    pub fn finish_discoverable_authentication(
        &self,
        credential: &PublicKeyCredential,
        state: DiscoverableAuthentication,
        credentials: &[DiscoverableKey],
    ) -> WebauthnResult<AuthenticationResult> {
        if state.rp_id != self.rp_id || state.rp_origin != self.rp_origin {
            return Err(WebauthnError::InvalidClientData);
        }
        let passkey = credentials
            .iter()
            .map(|key| &key.passkey)
            .find(|passkey| passkey.cred_id == credential.raw_id.as_slice())
            .ok_or(WebauthnError::CredentialNotFound)?;
        finish_authentication_with_passkey(
            credential,
            passkey,
            &state.challenge,
            self,
            BackupEligibilityPolicy::RejectBackupEligibilityChanges,
        )
    }

    fn request_options(
        &self,
        challenge: Vec<u8>,
        allow_credentials: Option<Vec<Vec<u8>>>,
    ) -> CredentialRequestOptions {
        CredentialRequestOptions {
            public_key: PublicKeyCredentialRequestOptions {
                challenge: challenge.into(),
                timeout: None,
                rp_id: Some(self.rp_id.clone()),
                allow_credentials: allow_credentials.map(|credentials| {
                    credentials
                        .into_iter()
                        .map(public_key_credential_descriptor)
                        .collect()
                }),
                user_verification: UserVerificationRequirement::Required,
                hints: None,
                attestation: AttestationConveyancePreference::None,
                attestation_formats: None,
                extensions: Some(authentication_extensions()),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyRegistration {
    challenge: Vec<u8>,
    user_uuid: Uuid,
    user_handle: Vec<u8>,
    rp_id: String,
    rp_origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyAuthentication {
    challenge: Vec<u8>,
    rp_id: String,
    rp_origin: String,
    allowed_credentials: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverableAuthentication {
    challenge: Vec<u8>,
    rp_id: String,
    rp_origin: String,
}

#[derive(Debug, Clone)]
pub struct DiscoverableKey {
    passkey: Passkey,
}

impl From<Passkey> for DiscoverableKey {
    fn from(passkey: Passkey) -> Self {
        Self { passkey }
    }
}

impl From<&Passkey> for DiscoverableKey {
    fn from(passkey: &Passkey) -> Self {
        Self {
            passkey: passkey.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Passkey {
    version: u8,
    cred_id: Vec<u8>,
    public_key_cose: Vec<u8>,
    user_handle: Vec<u8>,
    counter: u32,
    transports: Option<Vec<String>>,
    user_verified: bool,
    backup_eligible: bool,
    backup_state: bool,
}

impl Passkey {
    pub fn cred_id(&self) -> &Vec<u8> {
        &self.cred_id
    }

    pub fn update_credential(&mut self, result: &AuthenticationResult) -> Option<bool> {
        if result.cred_id != self.cred_id {
            return None;
        }
        debug_assert!(
            self.backup_eligible == result.backup_eligible
                || (!self.backup_eligible && result.backup_eligible && !result.backup_state),
            "backup eligibility policy must be validated before updating a passkey credential"
        );
        debug_assert!(
            !result.backup_state || self.backup_eligible,
            "backup state policy must be validated before updating a passkey credential"
        );

        let mut changed = false;
        if result.counter > self.counter {
            self.counter = result.counter;
            changed = true;
        }
        if result.backup_eligible && !self.backup_eligible {
            self.backup_eligible = true;
            changed = true;
        }
        if result.backup_state != self.backup_state {
            self.backup_state = result.backup_state;
            changed = true;
        }

        Some(changed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationResult {
    cred_id: Vec<u8>,
    counter: u32,
    user_verified: bool,
    backup_eligible: bool,
    backup_state: bool,
}

impl AuthenticationResult {
    pub fn cred_id(&self) -> &[u8] {
        &self.cred_id
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisterPublicKeyCredential {
    pub id: String,
    #[serde(rename = "rawId")]
    pub raw_id: Base64UrlSafeData,
    pub response: AuthenticatorAttestationResponseRaw,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default, alias = "clientExtensionResults", alias = "extensions")]
    pub extensions: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthenticatorAttestationResponseRaw {
    #[serde(rename = "attestationObject")]
    pub attestation_object: Base64UrlSafeData,
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: Base64UrlSafeData,
    #[serde(default)]
    pub transports: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublicKeyCredential {
    pub id: String,
    #[serde(rename = "rawId")]
    pub raw_id: Base64UrlSafeData,
    pub response: AuthenticatorAssertionResponseRaw,
    #[serde(default, alias = "clientExtensionResults")]
    pub extensions: serde_json::Value,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthenticatorAssertionResponseRaw {
    #[serde(rename = "authenticatorData")]
    pub authenticator_data: Base64UrlSafeData,
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: Base64UrlSafeData,
    pub signature: Base64UrlSafeData,
    #[serde(rename = "userHandle")]
    pub user_handle: Option<Base64UrlSafeData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Base64UrlSafeData(Vec<u8>);

impl Base64UrlSafeData {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for Base64UrlSafeData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for Base64UrlSafeData {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Base64UrlSafeData {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<Base64UrlSafeData> for Vec<u8> {
    fn from(value: Base64UrlSafeData) -> Self {
        value.0
    }
}

impl Serialize for Base64UrlSafeData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Base64UrlSafeData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Base64UrlVisitor;

        impl<'de> Visitor<'de> for Base64UrlVisitor {
            type Value = Base64UrlSafeData;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a base64url string or byte array")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = value.trim();
                URL_SAFE_NO_PAD
                    .decode(value)
                    .or_else(|_| base64::engine::general_purpose::STANDARD.decode(value))
                    .map(Base64UrlSafeData)
                    .map_err(|_| E::invalid_value(serde::de::Unexpected::Str(value), &self))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Base64UrlSafeData(value.to_vec()))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or_default());
                while let Some(byte) = seq.next_element()? {
                    bytes.push(byte);
                }
                Ok(Base64UrlSafeData(bytes))
            }
        }

        deserializer.deserialize_any(Base64UrlVisitor)
    }
}

#[derive(Serialize, Deserialize)]
struct PasskeyWire {
    version: u8,
    cred_id: Base64UrlSafeData,
    public_key_cose: Base64UrlSafeData,
    user_handle: Base64UrlSafeData,
    counter: u32,
    transports: Option<Vec<String>>,
    user_verified: bool,
    backup_eligible: bool,
    backup_state: bool,
}

impl Serialize for Passkey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PasskeyWire {
            version: self.version,
            cred_id: self.cred_id.clone().into(),
            public_key_cose: self.public_key_cose.clone().into(),
            user_handle: self.user_handle.clone().into(),
            counter: self.counter,
            transports: self.transports.clone(),
            user_verified: self.user_verified,
            backup_eligible: self.backup_eligible,
            backup_state: self.backup_state,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Passkey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let Ok(wire) = serde_json::from_value::<PasskeyWire>(value.clone()) {
            return Ok(Self {
                version: wire.version,
                cred_id: wire.cred_id.into(),
                public_key_cose: wire.public_key_cose.into(),
                user_handle: wire.user_handle.into(),
                counter: wire.counter,
                transports: wire.transports,
                user_verified: wire.user_verified,
                backup_eligible: wire.backup_eligible,
                backup_state: wire.backup_state,
            });
        }

        legacy_passkey_from_value(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy)]
enum BackupEligibilityPolicy {
    AllowFalseToTrueBackupEligibilityUpgrade,
    RejectBackupEligibilityChanges,
}

fn finish_authentication_with_passkey(
    credential: &PublicKeyCredential,
    passkey: &Passkey,
    challenge: &[u8],
    runtime: &Webauthn,
    backup_policy: BackupEligibilityPolicy,
) -> WebauthnResult<AuthenticationResult> {
    if credential.type_ != "public-key" {
        return Err(WebauthnError::InvalidCredential);
    }
    validate_client_data_json(
        credential.response.client_data_json.as_slice(),
        "webauthn.get",
        challenge,
        &runtime.rp_origin,
    )?;
    let auth_data = parse_authenticator_data(
        credential.response.authenticator_data.as_slice(),
        Some(&runtime.rp_id),
    )?;
    verify_es256_assertion_signature(
        &passkey.public_key_cose,
        credential.response.authenticator_data.as_slice(),
        credential.response.client_data_json.as_slice(),
        credential.response.signature.as_slice(),
    )?;
    validate_backup_eligibility(passkey, &auth_data, backup_policy)?;
    if (passkey.counter != 0 || auth_data.counter != 0) && auth_data.counter <= passkey.counter {
        return Err(WebauthnError::CounterReplay);
    }

    Ok(AuthenticationResult {
        cred_id: credential.raw_id.as_slice().to_vec(),
        counter: auth_data.counter,
        user_verified: auth_data.user_verified,
        backup_eligible: auth_data.backup_eligible,
        backup_state: auth_data.backup_state,
    })
}

fn validate_backup_eligibility(
    passkey: &Passkey,
    auth_data: &AuthenticatorData,
    policy: BackupEligibilityPolicy,
) -> WebauthnResult<()> {
    if passkey.backup_eligible != auth_data.backup_eligible {
        let allowed_upgrade = matches!(
            policy,
            BackupEligibilityPolicy::AllowFalseToTrueBackupEligibilityUpgrade
        ) && !passkey.backup_eligible
            && auth_data.backup_eligible
            && !auth_data.backup_state;
        if !allowed_upgrade {
            return Err(WebauthnError::CredentialBackupEligibilityInconsistent);
        }
    }

    if auth_data.backup_state && !passkey.backup_eligible {
        return Err(WebauthnError::CredentialMayNotBeHardwareBound);
    }

    Ok(())
}

fn public_key_credential_descriptor(id: Vec<u8>) -> PublicKeyCredentialDescriptor {
    PublicKeyCredentialDescriptor {
        ty: PublicKeyCredentialType::PublicKey,
        id: Bytes::from(id),
        transports: None,
    }
}

fn random_bytes(len: usize) -> WebauthnResult<Vec<u8>> {
    let mut bytes = vec![0_u8; len];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| WebauthnError::Random)?;
    Ok(bytes)
}

fn registration_extensions() -> AuthenticationExtensionsClientInputs {
    AuthenticationExtensionsClientInputs {
        cred_props: Some(true),
        cred_protect: Some(CredentialProtectionInput {
            credential_protection_policy: CredentialProtectionPolicy::UserVerificationRequired,
            enforce_credential_protection_policy: Some(false),
        }),
        uvm: Some(true),
    }
}

fn authentication_extensions() -> AuthenticationExtensionsClientInputs {
    AuthenticationExtensionsClientInputs {
        cred_props: None,
        cred_protect: None,
        uvm: Some(true),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientData {
    #[serde(rename = "type")]
    ty: String,
    challenge: String,
    origin: String,
    cross_origin: Option<bool>,
}

fn validate_client_data_json(
    client_data_json: &[u8],
    expected_type: &str,
    expected_challenge: &[u8],
    expected_origin: &str,
) -> WebauthnResult<()> {
    let client_data: ClientData =
        serde_json::from_slice(client_data_json).map_err(|_| WebauthnError::InvalidClientData)?;
    if client_data.ty != expected_type {
        return Err(WebauthnError::InvalidClientData);
    }
    if client_data.challenge != URL_SAFE_NO_PAD.encode(expected_challenge) {
        return Err(WebauthnError::InvalidClientData);
    }
    if client_data.origin != expected_origin {
        return Err(WebauthnError::InvalidClientData);
    }
    if client_data.cross_origin.unwrap_or(false) {
        return Err(WebauthnError::InvalidClientData);
    }
    Ok(())
}

#[derive(Debug)]
struct AttestationObject {
    fmt: String,
    auth_data: Vec<u8>,
    att_stmt_empty: bool,
}

fn parse_attestation_object(bytes: &[u8]) -> WebauthnResult<AttestationObject> {
    let value: CborValue = ciborium::de::from_reader(Cursor::new(bytes))
        .map_err(|_| WebauthnError::InvalidAttestationObject)?;
    let CborValue::Map(entries) = value else {
        return Err(WebauthnError::InvalidAttestationObject);
    };
    let fmt = cbor_text_field(&entries, "fmt")?;
    let auth_data = cbor_bytes_field(&entries, "authData")?;
    let att_stmt_empty = cbor_map_is_empty_field(&entries, "attStmt")?;

    Ok(AttestationObject {
        fmt,
        auth_data,
        att_stmt_empty,
    })
}

#[derive(Debug)]
struct AuthenticatorData {
    counter: u32,
    user_verified: bool,
    backup_eligible: bool,
    backup_state: bool,
    attested_credential: Option<AttestedCredential>,
}

#[derive(Debug)]
struct AttestedCredential {
    credential_id: Vec<u8>,
    public_key_cose: Vec<u8>,
}

// WebAuthn fixes rpIdHash and clientDataHash as SHA-256.
#[allow(clippy::disallowed_methods)]
fn parse_authenticator_data(
    authenticator_data: &[u8],
    expected_rp_id: Option<&str>,
) -> WebauthnResult<AuthenticatorData> {
    if authenticator_data.len() < 37 {
        return Err(WebauthnError::InvalidAuthenticatorData);
    }
    if let Some(rp_id) = expected_rp_id {
        let rp_id_hash = digest::digest(&digest::SHA256, rp_id.as_bytes());
        if authenticator_data[..32] != rp_id_hash.as_ref()[..] {
            return Err(WebauthnError::RpIdHashMismatch);
        }
    }

    let flags = authenticator_data[32];
    if flags & FLAG_UP == 0 {
        return Err(WebauthnError::UserPresenceRequired);
    }
    if flags & FLAG_BS != 0 && flags & FLAG_BE == 0 {
        return Err(WebauthnError::InvalidAuthenticatorData);
    }
    if flags & FLAG_UV == 0 {
        return Err(WebauthnError::UserVerificationRequired);
    }
    let counter = u32::from_be_bytes(
        authenticator_data[33..37]
            .try_into()
            .map_err(|_| WebauthnError::InvalidAuthenticatorData)?,
    );
    let attested_credential = if flags & FLAG_AT != 0 {
        Some(parse_attested_credential_data(
            &authenticator_data[37..],
            flags & FLAG_ED != 0,
        )?)
    } else {
        None
    };

    Ok(AuthenticatorData {
        counter,
        user_verified: flags & FLAG_UV != 0,
        backup_eligible: flags & FLAG_BE != 0,
        backup_state: flags & FLAG_BS != 0,
        attested_credential,
    })
}

fn parse_attested_credential_data(
    bytes: &[u8],
    has_extensions: bool,
) -> WebauthnResult<AttestedCredential> {
    if bytes.len() < 18 {
        return Err(WebauthnError::InvalidAuthenticatorData);
    }
    let credential_id_len = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
    let credential_id_start = 18;
    let credential_id_end = credential_id_start + credential_id_len;
    if bytes.len() <= credential_id_end {
        return Err(WebauthnError::InvalidAuthenticatorData);
    }
    let credential_id = bytes[credential_id_start..credential_id_end].to_vec();
    let mut cursor = Cursor::new(&bytes[credential_id_end..]);
    let cbor_key: CborValue =
        ciborium::de::from_reader(&mut cursor).map_err(|_| WebauthnError::InvalidPublicKey)?;
    let cose_key =
        CoseKey::from_cbor_value(cbor_key).map_err(|_| WebauthnError::InvalidPublicKey)?;
    let consumed = cursor.position() as usize;
    if !has_extensions && consumed != bytes[credential_id_end..].len() {
        return Err(WebauthnError::InvalidAuthenticatorData);
    }
    let public_key_cose = cose_key
        .to_vec()
        .map_err(|_| WebauthnError::InvalidPublicKey)?;

    Ok(AttestedCredential {
        credential_id,
        public_key_cose,
    })
}

fn validate_es256_cose_key(public_key_cose: &[u8]) -> WebauthnResult<()> {
    let cose_key =
        CoseKey::from_slice(public_key_cose).map_err(|_| WebauthnError::InvalidPublicKey)?;
    if !matches!(
        cose_key.alg,
        Some(coset::RegisteredLabelWithPrivate::Assigned(
            iana::Algorithm::ES256
        ))
    ) {
        return Err(WebauthnError::UnsupportedPublicKey);
    }
    cose_key
        .to_sec1_octet_string()
        .map_err(|_| WebauthnError::InvalidPublicKey)?;
    Ok(())
}

fn verify_es256_assertion_signature(
    public_key_cose: &[u8],
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature_bytes: &[u8],
) -> WebauthnResult<()> {
    let cose_key =
        CoseKey::from_slice(public_key_cose).map_err(|_| WebauthnError::InvalidPublicKey)?;
    if !matches!(
        cose_key.alg,
        Some(coset::RegisteredLabelWithPrivate::Assigned(
            iana::Algorithm::ES256
        ))
    ) {
        return Err(WebauthnError::UnsupportedPublicKey);
    }
    let sec1_public_key = cose_key
        .to_sec1_octet_string()
        .map_err(|_| WebauthnError::InvalidPublicKey)?;
    let verifying_key =
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, sec1_public_key);
    let signed_bytes = assertion_signed_bytes(authenticator_data, client_data_json);

    verifying_key
        .verify(&signed_bytes, signature_bytes)
        .map_err(|_| WebauthnError::InvalidSignature)
}

// WebAuthn fixes rpIdHash and clientDataHash as SHA-256.
#[allow(clippy::disallowed_methods)]
fn assertion_signed_bytes(authenticator_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
    let mut signed_bytes = Vec::with_capacity(authenticator_data.len() + 32);
    signed_bytes.extend_from_slice(authenticator_data);
    signed_bytes.extend_from_slice(digest::digest(&digest::SHA256, client_data_json).as_ref());
    signed_bytes
}

fn cbor_text_field(entries: &[(CborValue, CborValue)], name: &str) -> WebauthnResult<String> {
    entries
        .iter()
        .find_map(|(key, value)| {
            matches!(key, CborValue::Text(key) if key == name).then_some(value)
        })
        .and_then(|value| match value {
            CborValue::Text(value) => Some(value.clone()),
            _ => None,
        })
        .ok_or(WebauthnError::InvalidAttestationObject)
}

fn cbor_bytes_field(entries: &[(CborValue, CborValue)], name: &str) -> WebauthnResult<Vec<u8>> {
    entries
        .iter()
        .find_map(|(key, value)| {
            matches!(key, CborValue::Text(key) if key == name).then_some(value)
        })
        .and_then(|value| match value {
            CborValue::Bytes(value) => Some(value.clone()),
            _ => None,
        })
        .ok_or(WebauthnError::InvalidAttestationObject)
}

fn cbor_map_is_empty_field(entries: &[(CborValue, CborValue)], name: &str) -> WebauthnResult<bool> {
    entries
        .iter()
        .find_map(|(key, value)| {
            matches!(key, CborValue::Text(key) if key == name).then_some(value)
        })
        .and_then(|value| match value {
            CborValue::Map(entries) => Some(entries.is_empty()),
            _ => None,
        })
        .ok_or(WebauthnError::InvalidAttestationObject)
}

fn legacy_passkey_from_value(value: serde_json::Value) -> Result<Passkey, String> {
    let cred = value
        .get("cred")
        .ok_or_else(|| "missing legacy credential".to_string())?;
    let credential = cred
        .get("cred")
        .ok_or_else(|| "missing legacy public key".to_string())?;
    let algorithm = credential
        .get("type_")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing legacy algorithm".to_string())?;
    if algorithm != "ES256" {
        return Err(format!("unsupported legacy algorithm {algorithm}"));
    }
    let key = credential
        .get("key")
        .and_then(|value| value.get("EC_EC2"))
        .ok_or_else(|| "missing legacy ES256 key".to_string())?;
    let curve = key
        .get("curve")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing legacy key curve".to_string())?;
    if curve != "SECP256R1" {
        return Err(format!("unsupported legacy ES256 curve {curve}"));
    }
    let x = decode_json_base64_url_field(key, "x")?;
    let y = decode_json_base64_url_field(key, "y")?;
    let public_key_cose = CoseKeyBuilder::new_ec2_pub_key(iana::EllipticCurve::P_256, x, y)
        .algorithm(iana::Algorithm::ES256)
        .build()
        .to_vec()
        .map_err(|error| format!("could not encode legacy public key: {error}"))?;

    Ok(Passkey {
        version: 1,
        cred_id: decode_json_base64_url_field(cred, "cred_id")?,
        public_key_cose,
        user_handle: Vec::new(),
        counter: cred
            .get("counter")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_default(),
        transports: None,
        user_verified: cred
            .get("user_verified")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        backup_eligible: cred
            .get("backup_eligible")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        backup_state: cred
            .get("backup_state")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn decode_json_base64_url_field(value: &serde_json::Value, field: &str) -> Result<Vec<u8>, String> {
    let raw = value
        .get(field)
        .ok_or_else(|| format!("missing legacy field {field}"))?;
    serde_json::from_value::<Base64UrlSafeData>(raw.clone())
        .map(Vec::from)
        .map_err(|error| format!("invalid legacy byte field {field}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::{
        rand::SystemRandom,
        signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
    };

    // WebAuthn fixes rpIdHash and clientDataHash as SHA-256.
    #[allow(clippy::disallowed_methods)]
    fn authenticator_data_for(rp_id: &str, flags: u8, counter: u32) -> Vec<u8> {
        let mut authenticator_data = Vec::with_capacity(37);
        authenticator_data
            .extend_from_slice(digest::digest(&digest::SHA256, rp_id.as_bytes()).as_ref());
        authenticator_data.push(flags);
        authenticator_data.extend_from_slice(&counter.to_be_bytes());
        authenticator_data
    }

    fn es256_cose_public_key(key_pair: &EcdsaKeyPair) -> Vec<u8> {
        let public_key = key_pair.public_key().as_ref();
        CoseKeyBuilder::new_ec2_pub_key(
            iana::EllipticCurve::P_256,
            public_key[1..33].to_vec(),
            public_key[33..65].to_vec(),
        )
        .algorithm(iana::Algorithm::ES256)
        .build()
        .to_vec()
        .expect("serialize COSE key")
    }

    fn none_attestation_object_with_statement(auth_data: Vec<u8>, att_stmt: CborValue) -> Vec<u8> {
        let value = CborValue::Map(vec![
            (
                CborValue::Text("fmt".to_string()),
                CborValue::Text("none".to_string()),
            ),
            (
                CborValue::Text("authData".to_string()),
                CborValue::Bytes(auth_data),
            ),
            (CborValue::Text("attStmt".to_string()), att_stmt),
        ]);
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).expect("serialize attestation object");
        bytes
    }

    fn test_runtime() -> Webauthn {
        let origin = Url::parse("https://scryer.test").unwrap();
        WebauthnBuilder::new("scryer.test", &origin)
            .unwrap()
            .build()
            .unwrap()
    }

    fn test_passkey(
        key_pair: &EcdsaKeyPair,
        backup_eligible: bool,
        backup_state: bool,
        counter: u32,
        user_handle: Vec<u8>,
    ) -> Passkey {
        Passkey {
            version: 1,
            cred_id: b"cred".to_vec(),
            public_key_cose: es256_cose_public_key(key_pair),
            user_handle,
            counter,
            transports: None,
            user_verified: true,
            backup_eligible,
            backup_state,
        }
    }

    fn username_auth_state() -> PasskeyAuthentication {
        PasskeyAuthentication {
            challenge: b"authentication challenge".to_vec(),
            rp_id: "scryer.test".to_string(),
            rp_origin: "https://scryer.test".to_string(),
            allowed_credentials: vec![b"cred".to_vec()],
        }
    }

    fn discoverable_auth_state() -> DiscoverableAuthentication {
        DiscoverableAuthentication {
            challenge: b"authentication challenge".to_vec(),
            rp_id: "scryer.test".to_string(),
            rp_origin: "https://scryer.test".to_string(),
        }
    }

    fn signed_public_key_credential(
        key_pair: &EcdsaKeyPair,
        flags: u8,
        counter: u32,
        user_handle: Option<Vec<u8>>,
    ) -> PublicKeyCredential {
        let rng = SystemRandom::new();
        let authenticator_data = authenticator_data_for("scryer.test", flags, counter);
        let client_data_json = br#"{"type":"webauthn.get","challenge":"YXV0aGVudGljYXRpb24gY2hhbGxlbmdl","origin":"https://scryer.test"}"#;
        let signed_bytes = assertion_signed_bytes(&authenticator_data, client_data_json);
        let signature = key_pair.sign(&rng, &signed_bytes).expect("sign assertion");

        PublicKeyCredential {
            id: "Y3JlZA".to_string(),
            raw_id: b"cred".to_vec().into(),
            response: AuthenticatorAssertionResponseRaw {
                authenticator_data: authenticator_data.into(),
                client_data_json: client_data_json.to_vec().into(),
                signature: signature.as_ref().to_vec().into(),
                user_handle: user_handle.map(Into::into),
            },
            extensions: serde_json::Value::Object(Default::default()),
            type_: "public-key".to_string(),
        }
    }

    #[test]
    fn registration_options_are_browser_json_compatible() {
        let origin = Url::parse("https://scryer.test").unwrap();
        let runtime = WebauthnBuilder::new("scryer.test", &origin)
            .unwrap()
            .rp_name("Scryer")
            .build()
            .unwrap();
        let user_uuid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        let (options, state) = runtime
            .start_passkey_registration(user_uuid, "admin", "Admin", None)
            .unwrap();
        let payload = serde_json::to_value(&options).unwrap();

        assert_eq!(payload["publicKey"]["rp"]["id"], "scryer.test");
        assert_eq!(payload["publicKey"]["user"]["name"], "admin");
        assert!(payload["publicKey"]["challenge"].as_str().unwrap().len() > 20);
        assert_eq!(
            payload["publicKey"]["user"]["id"],
            URL_SAFE_NO_PAD.encode(user_uuid.as_bytes())
        );
        assert_eq!(
            payload["publicKey"]["authenticatorSelection"]["residentKey"],
            "required"
        );
        assert_eq!(
            payload["publicKey"]["authenticatorSelection"]["userVerification"],
            "required"
        );
        assert_eq!(payload["publicKey"]["extensions"]["credProps"], true);
        assert_eq!(payload["publicKey"]["extensions"]["uvm"], true);
        assert_eq!(
            payload["publicKey"]["extensions"]["credProtect"]["credentialProtectionPolicy"],
            "userVerificationRequired"
        );
        assert_eq!(
            payload["publicKey"]["extensions"]["credProtect"]["enforceCredentialProtectionPolicy"],
            false
        );
        assert_eq!(state.user_uuid, user_uuid);
    }

    #[test]
    fn authentication_options_are_browser_json_compatible() {
        let origin = Url::parse("https://scryer.test").unwrap();
        let runtime = WebauthnBuilder::new("scryer.test", &origin)
            .unwrap()
            .build()
            .unwrap();
        let passkey = Passkey {
            version: 1,
            cred_id: b"cred".to_vec(),
            public_key_cose: b"key".to_vec(),
            user_handle: b"user".to_vec(),
            counter: 0,
            transports: None,
            user_verified: false,
            backup_eligible: false,
            backup_state: false,
        };

        let (options, state) = runtime
            .start_passkey_authentication(&[passkey])
            .expect("start auth");
        let payload = serde_json::to_value(&options).unwrap();

        assert_eq!(payload["publicKey"]["rpId"], "scryer.test");
        assert_eq!(payload["publicKey"]["allowCredentials"][0]["id"], "Y3JlZA");
        assert_eq!(payload["publicKey"]["userVerification"], "required");
        assert_eq!(payload["publicKey"]["extensions"]["uvm"], true);
        assert!(
            payload["publicKey"]["extensions"]
                .as_object()
                .expect("extensions object")
                .get("credProtect")
                .is_none()
        );
        assert_eq!(state.allowed_credentials, vec![b"cred".to_vec()]);
    }

    #[test]
    fn client_data_and_authenticator_header_reject_tampering() {
        let challenge = b"authentication challenge";
        let client_data_json = br#"{"type":"webauthn.get","challenge":"YXV0aGVudGljYXRpb24gY2hhbGxlbmdl","origin":"https://scryer.test"}"#;
        let authenticator_data = authenticator_data_for("scryer.test", FLAG_UP | FLAG_UV, 42);

        validate_client_data_json(
            client_data_json,
            "webauthn.get",
            challenge,
            "https://scryer.test",
        )
        .expect("valid client data");
        let header = parse_authenticator_data(&authenticator_data, Some("scryer.test")).unwrap();
        assert_eq!(header.counter, 42);
        assert!(header.user_verified);

        assert!(matches!(
            validate_client_data_json(
                client_data_json,
                "webauthn.get",
                b"different challenge",
                "https://scryer.test",
            ),
            Err(WebauthnError::InvalidClientData)
        ));
        assert!(matches!(
            parse_authenticator_data(&authenticator_data, Some("other.test")),
            Err(WebauthnError::RpIdHashMismatch)
        ));
    }

    #[test]
    fn backup_state_requires_backup_eligible_flag() {
        let authenticator_data = authenticator_data_for("scryer.test", FLAG_UP | FLAG_BS, 0);

        assert!(matches!(
            parse_authenticator_data(&authenticator_data, Some("scryer.test")),
            Err(WebauthnError::InvalidAuthenticatorData)
        ));
    }

    #[test]
    fn user_verification_is_required() {
        let authenticator_data = authenticator_data_for("scryer.test", FLAG_UP, 0);

        assert!(matches!(
            parse_authenticator_data(&authenticator_data, Some("scryer.test")),
            Err(WebauthnError::UserVerificationRequired)
        ));
    }

    #[test]
    fn none_attestation_requires_empty_statement() {
        let valid = none_attestation_object_with_statement(
            b"authenticator data".to_vec(),
            CborValue::Map(vec![]),
        );
        parse_attestation_object(&valid).expect("empty none attestation statement");

        let invalid = none_attestation_object_with_statement(
            b"authenticator data".to_vec(),
            CborValue::Map(vec![(
                CborValue::Text("sig".to_string()),
                CborValue::Bytes(vec![1]),
            )]),
        );

        let parsed = parse_attestation_object(&invalid).expect("parse non-empty statement");
        assert!(!parsed.att_stmt_empty);
    }

    #[test]
    fn registration_rejects_non_empty_none_attestation_statement() {
        let origin = Url::parse("https://scryer.test").unwrap();
        let runtime = WebauthnBuilder::new("scryer.test", &origin)
            .unwrap()
            .build()
            .unwrap();
        let user_uuid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let (_, state) = runtime
            .start_passkey_registration(user_uuid, "admin", "Admin", None)
            .unwrap();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let public_key_cose = es256_cose_public_key(&key_pair);
        let credential_id = b"cred".to_vec();
        let mut auth_data = authenticator_data_for("scryer.test", FLAG_UP | FLAG_UV | FLAG_AT, 0);
        auth_data.extend_from_slice(&[0; 16]);
        auth_data.extend_from_slice(&(credential_id.len() as u16).to_be_bytes());
        auth_data.extend_from_slice(&credential_id);
        auth_data.extend_from_slice(&public_key_cose);
        let attestation_object = none_attestation_object_with_statement(
            auth_data,
            CborValue::Map(vec![(
                CborValue::Text("sig".to_string()),
                CborValue::Bytes(vec![1]),
            )]),
        );
        let client_data_json = serde_json::json!({
            "type": "webauthn.create",
            "challenge": URL_SAFE_NO_PAD.encode(&state.challenge),
            "origin": "https://scryer.test"
        })
        .to_string();
        let credential = RegisterPublicKeyCredential {
            id: URL_SAFE_NO_PAD.encode(&credential_id),
            raw_id: credential_id.into(),
            response: AuthenticatorAttestationResponseRaw {
                attestation_object: attestation_object.into(),
                client_data_json: client_data_json.into_bytes().into(),
                transports: None,
            },
            type_: "public-key".to_string(),
            extensions: serde_json::Value::Object(Default::default()),
        };

        assert!(matches!(
            runtime.finish_passkey_registration(&credential, &state),
            Err(WebauthnError::InvalidAttestationObject)
        ));
    }

    #[test]
    fn aws_lc_verifies_es256_assertion_signature_from_cose_key() {
        let rng = SystemRandom::new();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let public_key_cose = es256_cose_public_key(&key_pair);
        let authenticator_data = authenticator_data_for("scryer.test", FLAG_UP | FLAG_UV, 42);
        let client_data_json = br#"{"type":"webauthn.get","challenge":"YXV0aGVudGljYXRpb24gY2hhbGxlbmdl","origin":"https://scryer.test"}"#;
        let signed_bytes = assertion_signed_bytes(&authenticator_data, client_data_json);
        let signature = key_pair.sign(&rng, &signed_bytes).expect("sign assertion");

        verify_es256_assertion_signature(
            &public_key_cose,
            &authenticator_data,
            client_data_json,
            signature.as_ref(),
        )
        .expect("valid signature");

        let mut tampered = client_data_json.to_vec();
        tampered.extend_from_slice(b" ");
        assert!(matches!(
            verify_es256_assertion_signature(
                &public_key_cose,
                &authenticator_data,
                &tampered,
                signature.as_ref(),
            ),
            Err(WebauthnError::InvalidSignature)
        ));
    }

    #[test]
    fn counter_replay_is_rejected() {
        let origin = Url::parse("https://scryer.test").unwrap();
        let runtime = WebauthnBuilder::new("scryer.test", &origin)
            .unwrap()
            .build()
            .unwrap();
        let rng = SystemRandom::new();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let public_key_cose = es256_cose_public_key(&key_pair);
        let passkey = Passkey {
            version: 1,
            cred_id: b"cred".to_vec(),
            public_key_cose,
            user_handle: Uuid::nil().as_bytes().to_vec(),
            counter: 7,
            transports: None,
            user_verified: true,
            backup_eligible: false,
            backup_state: false,
        };
        let challenge = b"authentication challenge".to_vec();
        let client_data_json = br#"{"type":"webauthn.get","challenge":"YXV0aGVudGljYXRpb24gY2hhbGxlbmdl","origin":"https://scryer.test"}"#;
        let authenticator_data = authenticator_data_for("scryer.test", FLAG_UP | FLAG_UV, 0);
        let signed_bytes = assertion_signed_bytes(&authenticator_data, client_data_json);
        let signature = key_pair.sign(&rng, &signed_bytes).unwrap();
        let credential = PublicKeyCredential {
            id: "Y3JlZA".to_string(),
            raw_id: b"cred".to_vec().into(),
            response: AuthenticatorAssertionResponseRaw {
                authenticator_data: authenticator_data.into(),
                client_data_json: client_data_json.to_vec().into(),
                signature: signature.as_ref().to_vec().into(),
                user_handle: Some(Uuid::nil().as_bytes().to_vec().into()),
            },
            extensions: serde_json::Value::Object(Default::default()),
            type_: "public-key".to_string(),
        };

        assert!(matches!(
            finish_authentication_with_passkey(
                &credential,
                &passkey,
                &challenge,
                &runtime,
                BackupEligibilityPolicy::AllowFalseToTrueBackupEligibilityUpgrade,
            ),
            Err(WebauthnError::CounterReplay)
        ));
    }

    #[test]
    fn username_auth_allows_backup_eligible_upgrade_without_backup_state() {
        let runtime = test_runtime();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let user_handle = Uuid::nil().as_bytes().to_vec();
        let passkey = test_passkey(&key_pair, false, false, 0, user_handle.clone());
        let credential = signed_public_key_credential(
            &key_pair,
            FLAG_UP | FLAG_UV | FLAG_BE,
            1,
            Some(user_handle),
        );

        let result = runtime
            .finish_passkey_authentication(&credential, &username_auth_state(), &passkey)
            .expect("backup eligibility upgrade");

        assert!(result.backup_eligible);
        assert!(!result.backup_state);
        let mut updated = passkey.clone();
        assert_eq!(updated.update_credential(&result), Some(true));
        assert!(updated.backup_eligible);
        assert!(!updated.backup_state);
    }

    #[test]
    fn username_auth_rejects_backup_eligible_downgrade() {
        let runtime = test_runtime();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let user_handle = Uuid::nil().as_bytes().to_vec();
        let passkey = test_passkey(&key_pair, true, false, 0, user_handle.clone());
        let credential =
            signed_public_key_credential(&key_pair, FLAG_UP | FLAG_UV, 1, Some(user_handle));

        assert!(matches!(
            runtime.finish_passkey_authentication(&credential, &username_auth_state(), &passkey),
            Err(WebauthnError::CredentialBackupEligibilityInconsistent)
        ));
    }

    #[test]
    fn username_auth_rejects_backup_eligible_upgrade_with_backup_state() {
        let runtime = test_runtime();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let user_handle = Uuid::nil().as_bytes().to_vec();
        let passkey = test_passkey(&key_pair, false, false, 0, user_handle.clone());
        let credential = signed_public_key_credential(
            &key_pair,
            FLAG_UP | FLAG_UV | FLAG_BE | FLAG_BS,
            1,
            Some(user_handle),
        );

        assert!(matches!(
            runtime.finish_passkey_authentication(&credential, &username_auth_state(), &passkey),
            Err(WebauthnError::CredentialBackupEligibilityInconsistent)
        ));
    }

    #[test]
    fn discoverable_auth_rejects_backup_eligible_changes() {
        let runtime = test_runtime();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let passkey = test_passkey(&key_pair, false, false, 0, Uuid::nil().as_bytes().to_vec());
        let credential =
            signed_public_key_credential(&key_pair, FLAG_UP | FLAG_UV | FLAG_BE, 1, None);
        let discoverable = DiscoverableKey::from(passkey);

        assert!(matches!(
            runtime.finish_discoverable_authentication(
                &credential,
                discoverable_auth_state(),
                &[discoverable],
            ),
            Err(WebauthnError::CredentialBackupEligibilityInconsistent)
        ));
    }

    #[test]
    fn stored_backup_eligible_allows_backup_state_change() {
        let runtime = test_runtime();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let user_handle = Uuid::nil().as_bytes().to_vec();
        let passkey = test_passkey(&key_pair, true, false, 0, user_handle.clone());
        let credential = signed_public_key_credential(
            &key_pair,
            FLAG_UP | FLAG_UV | FLAG_BE | FLAG_BS,
            1,
            Some(user_handle),
        );

        let result = runtime
            .finish_passkey_authentication(&credential, &username_auth_state(), &passkey)
            .expect("backup state change");

        assert!(result.backup_eligible);
        assert!(result.backup_state);
        let mut updated = passkey.clone();
        assert_eq!(updated.update_credential(&result), Some(true));
        assert!(updated.backup_state);
    }

    #[test]
    fn username_auth_rejects_mismatched_user_handle_when_present() {
        let runtime = test_runtime();
        let key_pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate key");
        let passkey = test_passkey(&key_pair, false, false, 0, Uuid::nil().as_bytes().to_vec());
        let credential = signed_public_key_credential(
            &key_pair,
            FLAG_UP | FLAG_UV,
            1,
            Some(Uuid::max().as_bytes().to_vec()),
        );

        assert!(matches!(
            runtime.finish_passkey_authentication(&credential, &username_auth_state(), &passkey),
            Err(WebauthnError::InvalidCredential)
        ));
    }

    #[test]
    fn legacy_es256_passkey_json_deserializes() {
        let legacy_x = vec![0; 32];
        let legacy_y = vec![1; 32];
        let legacy = serde_json::json!({
            "cred": {
                "cred_id": "Y3JlZA",
                "cred": {
                    "type_": "ES256",
                    "key": {
                        "EC_EC2": {
                            "curve": "SECP256R1",
                            "x": legacy_x,
                            "y": legacy_y
                        }
                    }
                },
                "counter": 3,
                "user_verified": true,
                "backup_eligible": false,
                "backup_state": false
            }
        });

        let passkey: Passkey = serde_json::from_value(legacy).expect("legacy passkey");

        assert_eq!(passkey.cred_id, b"cred");
        assert_eq!(passkey.counter, 3);
        assert!(passkey.user_verified);
    }
}
