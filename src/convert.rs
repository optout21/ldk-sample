use bitcoin::hashes::hex::FromHex;
use bitcoin::BlockHash;
use lightning_block_sync::http::JsonResponse;
use std::convert::TryInto;

/*
// Removed, tx building (for funging tx) no longer needed from Btc Core
pub struct FundedTx {
	pub changepos: i64,
	pub hex: String,
}

impl TryInto<FundedTx> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<FundedTx> {
		Ok(FundedTx {
			changepos: self.0["changepos"].as_i64().unwrap(),
			hex: self.0["hex"].as_str().unwrap().to_string(),
		})
	}
}

pub struct RawTx(pub String);

impl TryInto<RawTx> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<RawTx> {
		Ok(RawTx(self.0.as_str().unwrap().to_string()))
	}
}

pub struct SignedTx {
	pub complete: bool,
	pub hex: String,
}

impl TryInto<SignedTx> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<SignedTx> {
		Ok(SignedTx {
			hex: self.0["hex"].as_str().unwrap().to_string(),
			complete: self.0["complete"].as_bool().unwrap(),
		})
	}
}
*/

pub struct NewAddress(pub String);
impl TryInto<NewAddress> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<NewAddress> {
		Ok(NewAddress(self.0.as_str().unwrap().to_string()))
	}
}

pub struct FeeResponse {
	pub feerate_sat_per_kw: Option<u32>,
	pub errored: bool,
}

impl TryInto<FeeResponse> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<FeeResponse> {
		let errored = !self.0["errors"].is_null();
		Ok(FeeResponse {
			errored,
			feerate_sat_per_kw: match self.0["feerate"].as_f64() {
				// Bitcoin Core gives us a feerate in BTC/KvB, which we need to convert to
				// satoshis/KW. Thus, we first multiply by 10^8 to get satoshis, then divide by 4
				// to convert virtual-bytes into weight units.
				Some(feerate_btc_per_kvbyte) => {
					Some((feerate_btc_per_kvbyte * 100_000_000.0 / 4.0).round() as u32)
				}
				None => None,
			},
		})
	}
}

pub struct BlockchainInfo {
	pub latest_height: usize,
	pub latest_blockhash: BlockHash,
	pub chain: String,
}

impl TryInto<BlockchainInfo> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<BlockchainInfo> {
		Ok(BlockchainInfo {
			latest_height: self.0["blocks"].as_u64().unwrap() as usize,
			latest_blockhash: BlockHash::from_hex(self.0["bestblockhash"].as_str().unwrap())
				.unwrap(),
			chain: self.0["chain"].as_str().unwrap().to_string(),
		})
	}
}

pub struct Utxo {
	pub tx_id: String,
	pub vout: u32,
	pub address: String,
	pub amount: f64,
	pub confirmations: u32,
	pub script_pub_key: String,  // the script key
	pub redeem_script: String,  // The redeemScript if scriptPubKey is P2SH (hex)
	pub witness_script: String,  // witnessScript if the scriptPubKey is P2WSH or P2SH-P2WSH
}

pub struct Unspents {
	pub utxos: Vec<Utxo>,
}

impl TryInto<Unspents> for JsonResponse {
	type Error = std::io::Error;
	fn try_into(self) -> std::io::Result<Unspents> {
		if !self.0.is_array() {
			return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "expected array"));
		}
		let mut arr: Vec<Utxo> = Vec::new();
		for e in self.0.as_array().unwrap() {
			arr.push(Utxo {
				tx_id: e["txid"].as_str().unwrap().to_string(),
				vout: e["vout"].as_u64().unwrap() as u32,
				address: e["address"].as_str().unwrap().to_string(),
				amount: e["amount"].as_f64().unwrap(),
				confirmations: e["confirmations"].as_u64().unwrap() as u32,
				script_pub_key: e["scriptPubKey"].as_str().unwrap().to_string(),
				redeem_script: match e["redeemScript"].as_str() { None => "".to_string(), Some(s) => s.to_string(), },
				witness_script: match e["witnessScript"].as_str() { None => "".to_string(), Some(s) => s.to_string(), },
			});
		}
		Ok(Unspents { utxos: arr })
	}
}
