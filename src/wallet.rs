use crate::convert::Unspents;
use crate::bitcoind_client::BitcoindClient;
use lightning::chain::keysinterface::{KeysInterface, KeysManager, KeyMaterial, Recipient, InMemorySigner, SpendableOutputDescriptor};
use lightning::chain::keysinterface::SpendableOutputDescriptor::{StaticOutput, DelayedPaymentOutput, StaticPaymentOutput};
use lightning::ln::msgs::DecodeError;
use lightning::ln::script::ShutdownScript;
use bitcoin::bech32::u5;
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::{Transaction, TxOut};
use bitcoin::network::constants::Network;
use bitcoin::secp256k1::{Secp256k1, SecretKey, ecdsa::RecoverableSignature, Signing};
use bitcoin::util::address::Address;
use bitcoin::util::key::{PublicKey, PrivateKey};
use bip39::Mnemonic;
use bip32::{XPrv, DerivationPath};
use std::sync::Arc;
use std::fs;

const PK_FILENAME: &str = ".pk_secret";
const LN_SEED_DERIVATION_PATH: &str = "m/2121'/9735'/0'/0/0";
const LN_SEED_DERIVATION_PATH_TESTNET: &str = "m/2121'/9735'/1'/0/0";

pub struct Wallet {
    network: Network,
    // own address
    pub address: String,
    // private key, private field
    private_key: Vec<u8>,
    pub public_key: Vec<u8>,
    pub utxos: Unspents,
    pub balance: f64,
    // 32-byte seed to be used by Lightning hot wallet, derived from mnemonic in a reproducible way
    pub seed_for_ldk: Vec<u8>,
}

pub struct WalletError(String);

impl From<bitcoin::util::address::Error> for WalletError {
    fn from(e: bitcoin::util::address::Error) -> Self {
        WalletError(e.to_string())
    }
}

impl From<bitcoin::util::key::Error> for WalletError {
    fn from(e: bitcoin::util::key::Error) -> Self {
        WalletError(e.to_string())
    }
}

impl ToString for WalletError {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

// given a mnemonic derive private keys and save them
pub fn import_wallet_mnemonic(mnemonic: &str, network: Network) -> Result<Wallet, WalletError> {
    let mnemonic = match Mnemonic::parse(mnemonic) {
        Err(e) => {
            println!("e {}", e.to_string());
            return Err(WalletError(format!("Invalid mnemonic {}", e.to_string())))
        },
        Ok(m) => m,
    };
	println!("Mnemonic is valid");
    let seed = mnemonic.to_seed("");
    let priv_key = priv_key_from_hdwallet(&seed, network)?;
	println!("Private key derived ({} bytes)", priv_key.len());

    let ldk_seed = ldk_seed_from_hdwallet(&seed, network)?;
    println!("LDK seed derived ({} bytes)", ldk_seed.len());

    if !save_private_keys(&priv_key, &ldk_seed) {
		let err = "Could not save private keys";
		println!("{}", err);
		return Err(WalletError(err.to_string()));
	}
	// check back
	match read_private_keys() {
		None => {
            let err = "Could not read back saved private keys";
			println!("{}", err);
			return Err(WalletError(err.to_string()));
		},
		Some((_, _)) => println!("Private keys saved"),
	}

    Wallet::from_pk(&priv_key, &ldk_seed, network)
}

pub fn load_wallet(network: Network) -> Result<Wallet, WalletError> {
    match read_private_keys() {
        None => {
            let err = format!("Could not read wallet (private keys, {})", PK_FILENAME);
		    println!("{}", err);
            return Err(WalletError(err));
        },
        Some((key1, key2)) => Wallet::from_pk(&key1, &key2, network),
    }
}

// Read the private keys from a file, 2x32 bytes, as hex string, concatenated
fn read_private_keys() -> Option<(Vec<u8>, Vec<u8>)> {
    let contents = fs::read_to_string(PK_FILENAME);
    match contents {
        Err(_e) => None,
        Ok(sraw) => {
            let s = sraw.trim();
            if s.len() < 2*2*32 {
                return None;
            }
            let key1_decode = hex::decode(s[0..2*32].to_string());
            match key1_decode {
                Err(_e) => None,
                Ok(key1) => {
                    let key2_decode = hex::decode(s[2*32..2*2*32].to_string());
                    match key2_decode {
                        Err(_e) => None,
                        Ok(key2) => {
                            Some((key1, key2))
                        },
                    }
                },
            }
        },
    }
}

fn save_private_keys(key1: &Vec<u8>, key2: &Vec<u8>) -> bool {
    let hex_string1 = hex::encode(key1);
    let hex_string2 = hex::encode(key2);
    match fs::write(PK_FILENAME, hex_string1.to_string() + &hex_string2) {
        Err(_) => return false,
        Ok(_) => return true,
    }
}

pub fn priv_key_from_hdwallet_with_derivation(seed: &[u8; 64], derivation_path: &str) -> Result<Vec<u8>, WalletError> {
    let dp = match derivation_path.parse::<DerivationPath>() {
        Err(e) => return Err(WalletError(format!("Invalid derivation_path {}", e.to_string()))),
        Ok(d) => d,
    };
    let child_xprv = match XPrv::derive_from_path(&seed, &dp) {
        Err(e) => return Err(WalletError(format!("Error deriving child key {}", e.to_string()))),
        Ok(k) => k,
    };
    let priv_key = child_xprv.private_key();
    Ok(priv_key.to_bytes().to_vec())
}

pub fn priv_key_from_hdwallet(seed: &[u8; 64], network: Network) -> Result<Vec<u8>, WalletError> {
    let derivation_path = if network == Network::Testnet { "m/84'/1'/0'/0/0" } else { "m/84'/0'/0'/0/0" };
    priv_key_from_hdwallet_with_derivation(seed, derivation_path)
}

pub fn ldk_seed_from_hdwallet(seed: &[u8; 64], network: Network) -> Result<Vec<u8>, WalletError> {
    let derivation_path = if network == Network::Testnet { LN_SEED_DERIVATION_PATH_TESTNET } else { LN_SEED_DERIVATION_PATH };
    priv_key_from_hdwallet_with_derivation(seed, derivation_path)
}

fn derive_pubkey_from_pk(priv_key: &Vec<u8>, network: Network) -> Result<Vec<u8>, WalletError> {
    let pk = PrivateKey::from_slice(&priv_key, network)?;
    let secp = Secp256k1::new();
    let public_key = PublicKey::from_private_key(&secp, &pk);
    Ok(public_key.to_bytes())
}

pub fn derive_address_from_pk(priv_key: &Vec<u8>, network: Network) -> Result<String, WalletError> {
    let pk = PrivateKey::from_slice(priv_key, network)?;
    let secp = Secp256k1::new();
    let public_key = PublicKey::from_private_key(&secp, &pk);
    let address = Address::p2wpkh(&public_key, network)?;
    Ok(address.to_string())
}

impl Wallet {
    pub fn from_pk(priv_key: &Vec<u8>, ldk_seed: &Vec<u8>, network: Network) -> Result<Wallet, WalletError> {
        let public_key = derive_pubkey_from_pk(&priv_key.clone(), network)?;
        let address = derive_address_from_pk(priv_key, network)?;
        Ok(Wallet {
            network,
            address,
            private_key: priv_key.clone(),
            public_key,
            utxos: Unspents { utxos: Vec::new() },
            balance: 0.0,
            seed_for_ldk: ldk_seed.clone(),
        })
    }

    pub fn print_address(&self) {
        println!("L1 wallet address: {}    pubkey:  {}", self.address, hex::encode(self.public_key.clone()));
    }

    pub fn print_balance(&self) {
        println!("L1 balance:  {}   utxos: {}", self.balance, self.utxos.utxos.len());
    }

    pub fn print(&self) {
        self.print_address();
        self.print_balance();
    }

    pub async fn retrieve_unspent(&self, bitcoind_client: &BitcoindClient) -> Unspents {
        bitcoind_client.list_unspent(0, self.address.as_str()).await
    }

    pub async fn retrieve_and_store_unspent(&mut self, bitcoind_client: &BitcoindClient)  {
        self.utxos = self.retrieve_unspent(bitcoind_client).await;
        self.balance = 0.0;
        for u in &self.utxos.utxos {
            self.balance += u.amount;
        }
    }

    pub fn create_send_tx(&self, to_address: &str, output_amount: u64) -> Vec<u8> {
        /* TODO
        let mut signing_input = Bitcoin::SigningInput::new();
        signing_input.hash_type = 1; // hashTypeAll
        signing_input.amount = output_amount as i64;
        signing_input.use_max_amount = false;
        signing_input.byte_fee = 1; // TODO
        signing_input.to_address = to_address.to_string();
        signing_input.change_address = self.address.clone();
        signing_input.coin_type = 0;
        signing_input.private_key.push(self.private_key.clone());

        let mut sum_amount: i64 = 0;
        for u in &self.utxos.utxos {
            if u.address != self.address {
                println!("discarding utxo, not own-address {} {}", u.address, self.address);
            } else {
                let mut utxo = Bitcoin::UnspentTransaction::new();
                let mut outpoint = Bitcoin::OutPoint::new();
                let mut hash = hex::decode(u.tx_id.clone()).unwrap();
                hash.reverse();
                outpoint.hash = hash;
                outpoint.index = u.vout;
                outpoint.sequence = u32::MAX - 1;
                utxo.out_point = ::protobuf::MessageField::some(outpoint);
                utxo.script = hex::decode(&u.script_pub_key).unwrap();
                let amount_sat = (u.amount * 100_000_000.0) as i64;
                utxo.amount = amount_sat;
                //println!("input utxo  '{}' '{}' '{}' {}", u.address, u.script_pub_key, u.witness_script, utxo.amount);
                signing_input.utxo.push(utxo);
                sum_amount += amount_sat;
            }
        }
        if signing_input.utxo.len() == 0 {
            println!("Error: 0 utxos to consider");
            return Vec::new();
        }
        if signing_input.amount - 1 >= sum_amount {
            signing_input.use_max_amount = true;
        }

        let input_ser = signing_input.write_to_bytes().unwrap();
        let input_ser_data = TWData::from_vec(&input_ser);

        let output_ser_data = any_signer_sign(&input_ser_data, 0);

        let outputp: Bitcoin::SigningOutput = protobuf::Message::parse_from_bytes(&output_ser_data.to_vec()).unwrap();

        //println!("tx encoded: {}", hex::encode(outputp.encoded.clone()));
        println!("tx tx_id:   {}", outputp.transaction_id);
        //println!("tx error:   {} {}", outputp.error.unwrap() as u16, outputp.error_message);

        outputp.encoded
        */
        Vec::new()
    }
}

// Replaces KeysManager, overriding get_shutdown_scriptpubkey()
pub struct WalletKeysManager {
    pub keys_manager: KeysManager,
    //wallet: Arc<Wallet>,
    shutdown_pubkey: bitcoin::secp256k1::PublicKey,
}

impl WalletKeysManager {
    pub fn new(wallet: &Arc<Wallet>, seed: &[u8; 32], starting_time_secs: u64, starting_time_nanos: u32) -> Self {
        WalletKeysManager {
            keys_manager: KeysManager::new(seed, starting_time_secs, starting_time_nanos),
            //wallet: wallet.clone(),
            shutdown_pubkey: bitcoin::secp256k1::PublicKey::from_slice(&wallet.public_key).unwrap(),
        }
    }

    /*
    fn derive_channel_keys(&self, channel_value_satoshis: u64, params: &[u8; 32]) -> InMemorySigner {
        self.keys_manager.derive_channel_keys(channel_value_satoshis, params)
    }
    */

    pub fn spend_spendable_outputs<C: Signing>(&self, descriptors: &[&SpendableOutputDescriptor], outputs: Vec<TxOut>, change_destination_script: Script, feerate_sat_per_1000_weight: u32, secp_ctx: &Secp256k1<C>) -> Option<Result<Transaction, ()>> {
        let shutdown_script: Script = ShutdownScript::new_p2wpkh_from_pubkey(self.shutdown_pubkey).into_inner();
        let mut is_any_different = false;
		for out in descriptors {
            let output = match out {
                StaticOutput { outpoint: _, output } => output,
                DelayedPaymentOutput(delayed) => &delayed.output,
                StaticPaymentOutput(static_o) => &static_o.output,
            };
            is_any_different |= output.script_pubkey != shutdown_script;
        }

        if !is_any_different {
            // output(s) is the shutdown pubkey, which does not need a sweep transfer
            println!("Output(s) became spendable, but it is (all are) to shutdown pubkey, no sweep tx needed ({})", descriptors.len());
            None
        } else {
            Some(self.keys_manager.spend_spendable_outputs(descriptors, outputs, change_destination_script, feerate_sat_per_1000_weight, secp_ctx))
        }
    }
}

impl KeysInterface for WalletKeysManager {
    type Signer = InMemorySigner;

	fn get_node_secret(&self, recipient: Recipient) -> Result<SecretKey, ()> {
        self.keys_manager.get_node_secret(recipient)
    }

	fn get_destination_script(&self) -> Script {
        self.keys_manager.get_destination_script()
    }

	fn get_shutdown_scriptpubkey(&self) -> ShutdownScript {
        // Overriden behavior: use 'external' L1 wallet address here, instead of shutdown address derived from LDK master key
        //self.keys_manager.get_shutdown_scriptpubkey()
        //let pubkey = bitcoin::secp256k1::PublicKey::from_slice(&self.wallet.public_key).unwrap();
        ShutdownScript::new_p2wpkh_from_pubkey(self.shutdown_pubkey)
    }

    fn get_channel_signer(&self, inbound: bool, channel_value_satoshis: u64) -> Self::Signer {
        self.keys_manager.get_channel_signer(inbound, channel_value_satoshis)
    }

    fn get_secure_random_bytes(&self) -> [u8; 32] {
        self.keys_manager.get_secure_random_bytes()
    }

	fn read_chan_signer(&self, reader: &[u8]) -> Result<Self::Signer, DecodeError> {
        self.keys_manager.read_chan_signer(reader)
    }

	fn sign_invoice(&self, hrp_bytes: &[u8], invoice_data: &[u5], receipient: Recipient) -> Result<RecoverableSignature, ()> {
        self.keys_manager.sign_invoice(hrp_bytes, invoice_data, receipient)
    }

	fn get_inbound_payment_key_material(&self) -> KeyMaterial {
        self.keys_manager.get_inbound_payment_key_material()
    }
}
