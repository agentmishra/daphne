// Copyright (c) 2022 Cloudflare, Inc. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause

//! Messages in the DAP protocol.

use crate::{DapAbort, DapTaskConfig};
use prio::codec::{decode_u16_items, encode_u16_items, CodecError, Decode, Encode};
use serde::{Deserialize, Serialize};
use std::{
    convert::{TryFrom, TryInto},
    fmt::Debug,
    io::{Cursor, Read},
};

const KEM_ID_X25519_HKDF_SHA256: u16 = 0x0020;
const KDF_ID_HKDF_SHA256: u16 = 0x0001;
const AEAD_ID_AES128GCM: u16 = 0x0001;

/// The identifier for a DAP task.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Id(#[serde(with = "hex")] pub [u8; 32]);

impl Id {
    /// Return the URL-safe, base64 encoding of the task ID.
    pub fn to_base64url(&self) -> String {
        base64::encode_config(&self.0, base64::URL_SAFE_NO_PAD)
    }
}

impl Encode for Id {
    fn encode(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.0);
    }
}

impl Decode for Id {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        let mut data = [0; 32];
        bytes.read_exact(&mut data[..])?;
        Ok(Id(data))
    }
}

impl AsRef<[u8]> for Id {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// The timestamp and random number fields of a [`Report`].
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[allow(missing_docs)]
pub struct Nonce {
    pub time: u64,
    pub rand: u64,
}

impl Encode for Nonce {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.time.encode(bytes);
        self.rand.encode(bytes);
    }
}

impl Decode for Nonce {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Nonce {
            time: u64::decode(bytes)?,
            rand: u64::decode(bytes)?,
        })
    }
}

/// A report generated by a client.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[allow(missing_docs)]
pub struct Report {
    pub task_id: Id,
    pub nonce: Nonce,
    pub(crate) ignored_extensions: Vec<u8>,
    pub encrypted_input_shares: Vec<HpkeCiphertext>,
}

impl Encode for Report {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.task_id.encode(bytes);
        self.nonce.encode(bytes);
        encode_u16_bytes(bytes, &self.ignored_extensions);
        encode_u16_items(bytes, &(), &self.encrypted_input_shares);
    }
}

impl Decode for Report {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            task_id: Id::decode(bytes)?,
            nonce: Nonce::decode(bytes)?,
            ignored_extensions: decode_u16_bytes(bytes)?,
            encrypted_input_shares: decode_u16_items(&(), bytes)?,
        })
    }
}

/// An initial aggregate sub-request sent in an [`AggregateInitReq`]. The contents of this
/// structure pertain to a single report.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[allow(missing_docs)]
pub struct ReportShare {
    pub nonce: Nonce,
    pub(crate) ignored_extensions: Vec<u8>,
    pub encrypted_input_share: HpkeCiphertext,
}

impl Encode for ReportShare {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.nonce.encode(bytes);
        encode_u16_bytes(bytes, &self.ignored_extensions);
        self.encrypted_input_share.encode(bytes);
    }
}

impl Decode for ReportShare {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            nonce: Nonce::decode(bytes)?,
            ignored_extensions: decode_u16_bytes(bytes)?,
            encrypted_input_share: HpkeCiphertext::decode(bytes)?,
        })
    }
}

/// Aggregate request.
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateReq {
    pub task_id: Id,
    pub agg_job_id: Id,
    pub var: AggregateReqVar,
}

impl AggregateReq {
    pub(crate) fn get_report_shares_ref(&self) -> Result<&[ReportShare], DapAbort> {
        match &self.var {
            AggregateReqVar::Init { agg_param: _, seq } => Ok(seq),
            _ => Err(DapAbort::UnrecognizedMessage),
        }
    }

    pub(crate) fn get_transitions_ref(&self) -> Result<&[Transition], DapAbort> {
        match &self.var {
            AggregateReqVar::Continue { seq } => Ok(seq),
            _ => Err(DapAbort::UnrecognizedMessage),
        }
    }

    #[cfg(test)]
    pub(crate) fn unwrap_agg_param_ref(&self) -> &[u8] {
        assert_matches::assert_matches!(&self.var, AggregateReqVar::Init{ agg_param, .. } => agg_param)
    }
}

impl Encode for AggregateReq {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.task_id.encode(bytes);
        self.agg_job_id.encode(bytes);
        self.var.encode(bytes);
    }
}

impl Decode for AggregateReq {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            task_id: Id::decode(bytes)?,
            agg_job_id: Id::decode(bytes)?,
            var: AggregateReqVar::decode(bytes)?,
        })
    }
}

/// Aggregate request variant.
#[derive(Clone, Debug, PartialEq)]
pub enum AggregateReqVar {
    Init {
        agg_param: Vec<u8>,
        seq: Vec<ReportShare>,
    },
    Continue {
        seq: Vec<Transition>,
    },
}

impl Default for AggregateReqVar {
    fn default() -> Self {
        Self::Init {
            agg_param: Vec::default(),
            seq: Vec::default(),
        }
    }
}

impl Encode for AggregateReqVar {
    fn encode(&self, bytes: &mut Vec<u8>) {
        match self {
            AggregateReqVar::Init { agg_param, seq } => {
                0_u8.encode(bytes);
                encode_u16_bytes(bytes, agg_param);
                encode_u16_items(bytes, &(), seq);
            }
            AggregateReqVar::Continue { seq } => {
                1_u8.encode(bytes);
                encode_u16_items(bytes, &(), seq);
            }
        }
    }
}

impl Decode for AggregateReqVar {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        match u8::decode(bytes)? {
            0 => Ok(Self::Init {
                agg_param: decode_u16_bytes(bytes)?,
                seq: decode_u16_items(&(), bytes)?,
            }),
            1 => Ok(Self::Continue {
                seq: decode_u16_items(&(), bytes)?,
            }),
            _ => Err(CodecError::UnexpectedValue),
        }
    }
}

/// Transition message.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub nonce: Nonce,
    pub var: TransitionVar,
}

impl Encode for Transition {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.nonce.encode(bytes);
        self.var.encode(bytes);
    }
}

impl Decode for Transition {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            nonce: Nonce::decode(bytes)?,
            var: TransitionVar::decode(bytes)?,
        })
    }
}

/// Transition message variant.
#[derive(Clone, Debug, PartialEq)]
pub enum TransitionVar {
    Continued(Vec<u8>),
    Finished,
    Failed(TransitionFailure),
}

impl Encode for TransitionVar {
    fn encode(&self, bytes: &mut Vec<u8>) {
        match self {
            TransitionVar::Continued(vdaf_message) => {
                0_u8.encode(bytes);
                encode_u16_bytes(bytes, vdaf_message);
            }
            TransitionVar::Finished => {
                1_u8.encode(bytes);
            }
            TransitionVar::Failed(err) => {
                2_u8.encode(bytes);
                err.encode(bytes);
            }
        }
    }
}

impl Decode for TransitionVar {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        match u8::decode(bytes)? {
            0 => Ok(Self::Continued(decode_u16_bytes(bytes)?)),
            1 => Ok(Self::Finished),
            2 => Ok(Self::Failed(TransitionFailure::decode(bytes)?)),
            _ => Err(CodecError::UnexpectedValue),
        }
    }
}

/// Transition error.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub enum TransitionFailure {
    BatchCollected = 0,
    ReportReplayed = 1,
    ReportDropped = 2,
    HpkeUnknownConfigId = 3,
    HpkeDecryptError = 4,
    VdafPrepError = 5,
}

impl TryFrom<u8> for TransitionFailure {
    type Error = CodecError;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            b if b == Self::BatchCollected as u8 => Ok(Self::BatchCollected),
            b if b == Self::ReportReplayed as u8 => Ok(Self::ReportReplayed),
            b if b == Self::ReportDropped as u8 => Ok(Self::ReportDropped),
            b if b == Self::HpkeUnknownConfigId as u8 => Ok(Self::HpkeUnknownConfigId),
            b if b == Self::HpkeDecryptError as u8 => Ok(Self::HpkeDecryptError),
            b if b == Self::VdafPrepError as u8 => Ok(Self::VdafPrepError),
            _ => Err(CodecError::UnexpectedValue),
        }
    }
}

impl Encode for TransitionFailure {
    fn encode(&self, bytes: &mut Vec<u8>) {
        (*self as u8).encode(bytes);
    }
}

impl Decode for TransitionFailure {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        u8::decode(bytes)?.try_into()
    }
}

impl std::fmt::Display for TransitionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::BatchCollected => write!(f, "batch-collected({})", *self as u8),
            Self::ReportReplayed => write!(f, "report-replayed({})", *self as u8),
            Self::ReportDropped => write!(f, "report-dropped({})", *self as u8),
            Self::HpkeUnknownConfigId => write!(f, "hpke-unknown-config-id({})", *self as u8),
            Self::HpkeDecryptError => write!(f, "hpke-decrypt-error({})", *self as u8),
            Self::VdafPrepError => write!(f, "vdaf-prep-error({})", *self as u8),
        }
    }
}

/// An aggregate response sent from the Helper to the Leader in response to an (initial) aggregate
/// request. The contents of this structure pertain to a single task and batch.
#[derive(Debug, PartialEq, Default)]
#[allow(missing_docs)]
pub struct AggregateResp {
    pub seq: Vec<Transition>,
}

impl Encode for AggregateResp {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_u16_items(bytes, &(), &self.seq);
    }
}

impl Decode for AggregateResp {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            seq: decode_u16_items(&(), bytes)?,
        })
    }
}

/// A batch interval.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[allow(missing_docs)]
pub struct Interval {
    pub start: u64,
    pub duration: u64,
}

impl Interval {
    /// Return the end of the interval, i.e., `self.start + self.duration`.
    pub fn end(&self) -> u64 {
        self.start + self.duration
    }

    /// Check that the batch interval is valid for the given task configuration.
    pub fn is_valid_for(&self, task_config: &DapTaskConfig) -> bool {
        if self.start % task_config.min_batch_duration != 0
            || self.duration % task_config.min_batch_duration != 0
            || self.duration < task_config.min_batch_duration
        {
            return false;
        }
        true
    }
}

impl Encode for Interval {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.start.encode(bytes);
        self.duration.encode(bytes);
    }
}

impl Decode for Interval {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            start: u64::decode(bytes)?,
            duration: u64::decode(bytes)?,
        })
    }
}

/// A collect request.
//
// TODO Add serialization tests.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CollectReq {
    pub task_id: Id,
    pub batch_interval: Interval,
    pub agg_param: Vec<u8>,
}

impl Encode for CollectReq {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.task_id.encode(bytes);
        self.batch_interval.encode(bytes);
        encode_u16_bytes(bytes, &self.agg_param);
    }
}

impl Decode for CollectReq {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            task_id: Id::decode(bytes)?,
            batch_interval: Interval::decode(bytes)?,
            agg_param: decode_u16_bytes(bytes)?,
        })
    }
}

/// A collect response.
//
// TODO Add serialization tests.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CollectResp {
    pub encrypted_agg_shares: Vec<HpkeCiphertext>,
}

impl Encode for CollectResp {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_u16_items(bytes, &(), &self.encrypted_agg_shares);
    }
}

impl Decode for CollectResp {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            encrypted_agg_shares: decode_u16_items(&(), bytes)?,
        })
    }
}

/// An aggregate-share request.
//
// TODO Add serialization tests.
#[derive(Debug, Default)]
pub struct AggregateShareReq {
    pub task_id: Id,
    pub batch_interval: Interval,
    pub agg_param: Vec<u8>,
    pub report_count: u64,
    pub checksum: [u8; 32],
}

impl Encode for AggregateShareReq {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.task_id.encode(bytes);
        self.batch_interval.encode(bytes);
        encode_u16_bytes(bytes, &self.agg_param);
        self.report_count.encode(bytes);
        bytes.extend_from_slice(&self.checksum);
    }
}

impl Decode for AggregateShareReq {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            task_id: Id::decode(bytes)?,
            batch_interval: Interval::decode(bytes)?,
            agg_param: decode_u16_bytes(bytes)?,
            report_count: u64::decode(bytes)?,
            checksum: {
                let mut checksum = [0u8; 32];
                bytes.read_exact(&mut checksum[..])?;
                checksum
            },
        })
    }
}

/// An aggregate-share response.
//
// TODO Add serialization tests.
#[derive(Debug)]
pub struct AggregateShareResp {
    pub encrypted_agg_share: HpkeCiphertext,
}

impl Encode for AggregateShareResp {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.encrypted_agg_share.encode(bytes);
    }
}

impl Decode for AggregateShareResp {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            encrypted_agg_share: HpkeCiphertext::decode(bytes)?,
        })
    }
}

/// Codepoint for KEM schemes compatible with HPKE.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HpkeKemId {
    X25519HkdfSha256,
    NotImplemented(u16),
}

impl From<HpkeKemId> for u16 {
    fn from(kem_id: HpkeKemId) -> Self {
        match kem_id {
            HpkeKemId::X25519HkdfSha256 => KEM_ID_X25519_HKDF_SHA256,
            HpkeKemId::NotImplemented(x) => x,
        }
    }
}

impl Encode for HpkeKemId {
    fn encode(&self, bytes: &mut Vec<u8>) {
        u16::from(*self).encode(bytes);
    }
}

impl Decode for HpkeKemId {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        match u16::decode(bytes)? {
            x if x == KEM_ID_X25519_HKDF_SHA256 => Ok(Self::X25519HkdfSha256),
            x => Ok(Self::NotImplemented(x)),
        }
    }
}

/// Codepoint for KDF schemes compatible with HPKE.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HpkeKdfId {
    HkdfSha256,
    NotImplemented(u16),
}

impl From<HpkeKdfId> for u16 {
    fn from(kdf_id: HpkeKdfId) -> Self {
        match kdf_id {
            HpkeKdfId::HkdfSha256 => KDF_ID_HKDF_SHA256,
            HpkeKdfId::NotImplemented(x) => x,
        }
    }
}

impl Encode for HpkeKdfId {
    fn encode(&self, bytes: &mut Vec<u8>) {
        u16::from(*self).encode(bytes);
    }
}

impl Decode for HpkeKdfId {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        match u16::decode(bytes)? {
            x if x == KDF_ID_HKDF_SHA256 => Ok(Self::HkdfSha256),
            x => Ok(Self::NotImplemented(x)),
        }
    }
}

/// Codepoint for AEAD schemes compatible with HPKE.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HpkeAeadId {
    Aes128Gcm,
    NotImplemented(u16),
}

impl From<HpkeAeadId> for u16 {
    fn from(aead_id: HpkeAeadId) -> Self {
        match aead_id {
            HpkeAeadId::Aes128Gcm => AEAD_ID_AES128GCM,
            HpkeAeadId::NotImplemented(x) => x,
        }
    }
}

impl Encode for HpkeAeadId {
    fn encode(&self, bytes: &mut Vec<u8>) {
        u16::from(*self).encode(bytes);
    }
}

impl Decode for HpkeAeadId {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        match u16::decode(bytes)? {
            x if x == AEAD_ID_AES128GCM => Ok(Self::Aes128Gcm),
            x => Ok(Self::NotImplemented(x)),
        }
    }
}

/// The HPKE public key configuration of a Server.
#[derive(Clone, Debug, PartialEq)]
pub struct HpkeConfig {
    pub id: u8,
    pub kem_id: HpkeKemId,
    pub kdf_id: HpkeKdfId,
    pub aead_id: HpkeAeadId,
    // TODO Change this type to be the deserialized public key in order to avoid copying the
    // serialized key. We can't do this with rust-hpke because <X25519HkdfSha256 as Kem>::PublicKey
    // doesn't implement Debug. Eventually we'll replace rust-hpke with a more ergonomic
    // implementation that does. For now we'll eat the copy.
    pub(crate) public_key: Vec<u8>,
}

impl Encode for HpkeConfig {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.id.encode(bytes);
        self.kem_id.encode(bytes);
        self.kdf_id.encode(bytes);
        self.aead_id.encode(bytes);
        encode_u16_bytes(bytes, &self.public_key);
    }
}

impl Decode for HpkeConfig {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            id: u8::decode(bytes)?,
            kem_id: HpkeKemId::decode(bytes)?,
            kdf_id: HpkeKdfId::decode(bytes)?,
            aead_id: HpkeAeadId::decode(bytes)?,
            public_key: decode_u16_bytes(bytes)?,
        })
    }
}

/// An HPKE ciphertext. In the DAP protocol, input shares and aggregate shares are encrypted to the
/// intended recipient.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[allow(missing_docs)]
pub struct HpkeCiphertext {
    pub config_id: u8,
    #[serde(with = "hex")]
    pub enc: Vec<u8>,
    #[serde(with = "hex")]
    pub payload: Vec<u8>,
}

impl Encode for HpkeCiphertext {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.config_id.encode(bytes);
        encode_u16_bytes(bytes, &self.enc);
        encode_u16_bytes(bytes, &self.payload);
    }
}

impl Decode for HpkeCiphertext {
    fn decode(bytes: &mut Cursor<&[u8]>) -> Result<Self, CodecError> {
        Ok(Self {
            config_id: u8::decode(bytes)?,
            enc: decode_u16_bytes(bytes)?,
            payload: decode_u16_bytes(bytes)?,
        })
    }
}

// NOTE ring provides a similar function, but as of version 0.16.20, it doesn't compile to
// wasm32-unknown-unknown.
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut r = 0;
    for (x, y) in left.iter().zip(right) {
        r |= x ^ y;
    }
    r == 0
}

pub(crate) fn encode_u16_bytes(bytes: &mut Vec<u8>, input: &[u8]) {
    let len: u16 = input.len().try_into().unwrap();
    len.encode(bytes);
    bytes.extend_from_slice(input);
}

fn decode_u16_bytes(bytes: &mut Cursor<&[u8]>) -> Result<Vec<u8>, CodecError> {
    let len = u16::decode(bytes)? as usize;
    let mut out = vec![0; len];
    bytes.read_exact(&mut out)?;
    Ok(out)
}
