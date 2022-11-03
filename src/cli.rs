use crate::disk;
use crate::hex_utils;
use crate::{
	ChannelManager, HTLCStatus, InvoicePayer, MillisatAmount, NetworkGraph, PaymentInfo,
	PaymentInfoStorage, PeerManager,
};
use crate::ChainMonitor;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use bitcoin::network::constants::Network;
use bitcoin::secp256k1::PublicKey;
use lightning::chain::keysinterface::{KeysInterface, KeysManager, Recipient, InMemorySigner};
use lightning::chain::channelmonitor::{Balance, ChannelMonitor};
//use lightning::ln::msgs::NetAddress;
use lightning::ln::{PaymentHash, PaymentPreimage};
use lightning::routing::gossip::NodeId;
use lightning::util::config::{ChannelHandshakeConfig, ChannelHandshakeLimits, UserConfig};
use lightning::util::events::EventHandler;
use lightning_invoice::payment::PaymentError;
use lightning_invoice::{utils, Currency, Invoice};
use std::env;
use std::io;
use std::io::{BufRead, Write};
use std::net::{SocketAddr, ToSocketAddrs}; // IpAddr
use std::ops::Deref;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::wallet::*;
use crate::env::*;

/*
// Removed, moved to `env`
pub(crate) struct LdkUserInfo {
	pub(crate) bitcoind_rpc_username: String,
	pub(crate) bitcoind_rpc_password: String,
	pub(crate) bitcoind_rpc_port: u16,
	pub(crate) bitcoind_rpc_host: String,
	pub(crate) ldk_storage_dir_path: String,
	pub(crate) ldk_peer_listening_port: u16,
	pub(crate) ldk_announced_listen_addr: Vec<NetAddress>,
	pub(crate) ldk_announced_node_name: [u8; 32],
	pub(crate) network: Network,
}
*/

// Handle importwallet option. Return true if this otion was detected (regardless of the outcome)
pub(crate) fn handle_import_wallet(network: Network) -> bool {
	let mut is_import = false;
	if env::args().len() < 2 {
		return is_import;
	}
	if env::args().skip(1).next().unwrap() != "importwallet" {
		return is_import;
	}
	is_import = true;

	// ask for mnemonic interactively
	print!("Enter mnemonic (12-24 words): ");
	io::stdout().flush().unwrap(); // Without flushing, the `>` doesn't print
	let stdin = io::stdin();
	let mut line_reader = stdin.lock().lines();
	let mnemonic = match line_reader.next() {
		Some(l) => l.unwrap(),
		None => return is_import,
	};

	let wallet = match import_wallet_mnemonic(&mnemonic, network) {
		None => return is_import,
		Some(wallet) => wallet,
	};

    wallet.print_address();

	is_import
}

// Print out current status, balances, channels, etc.
pub(crate) fn print_status_balance(wallet: &Arc<Wallet>, channel_manager: &ChannelManager, chain_monitor: &ChainMonitor, channelmonitors: &Vec<(BlockHash, ChannelMonitor<InMemorySigner>)>, include_l1: bool) {
	if include_l1 {
		wallet.print();
	}

	list_channel_balances(&channel_manager);

	let balances = chain_monitor.get_claimable_balances(&[]);
	print_balances("chain_monitor", &balances);
	for (_, channel_monitor) in channelmonitors.iter() {
		let balances = channel_monitor.get_claimable_balances();
		print_balances("channel", &balances);
	}	
}

fn print_balances(name: &str, balances: &Vec<Balance>) {
	print!("Balances in {}: ({})   ", name, balances.len());
	for b in balances {
		let bal = match b {
			Balance::ClaimableOnChannelClose{ claimable_amount_satoshis } => claimable_amount_satoshis,
			Balance::ClaimableAwaitingConfirmations { claimable_amount_satoshis, confirmation_height: _ } => claimable_amount_satoshis,
			Balance::ContentiousClaimable { claimable_amount_satoshis, timeout_height: _ } => claimable_amount_satoshis,
			Balance::MaybeClaimableHTLCAwaitingTimeout { claimable_amount_satoshis, claimable_height: _ } => claimable_amount_satoshis,
		};
		print!("{} ", bal);
	}
	println!("");
}

fn list_channel_balances(channel_manager: &ChannelManager) {
	let channels = &channel_manager.list_channels();
	if channels.len() == 0 {
		println!("No open channels");
		return;
	}
	for c in channels {
		println!("{} open channels:", channels.len());
		print!("  val {}  bal {}  ", c.channel_value_satoshis, c.balance_msat);
		match c.short_channel_id {
			Some(id) => { print!("{} ", id); },
			None => {},
		};
		println!("");
	}
}

/*
// Removed, moved to `env`
pub(crate) fn parse_startup_args() -> Result<LdkUserInfo, ()> {
	if env::args().len() < 3 {
		println!("ldk-tutorial-node requires 3 arguments: `cargo run <bitcoind-rpc-username>:<bitcoind-rpc-password>@<bitcoind-rpc-host>:<bitcoind-rpc-port> ldk_storage_directory_path [<ldk-incoming-peer-listening-port>] [bitcoin-network] [announced-node-name announced-listen-addr*]`");
		return Err(());
	}
	let bitcoind_rpc_info = env::args().skip(1).next().unwrap();
	let bitcoind_rpc_info_parts: Vec<&str> = bitcoind_rpc_info.rsplitn(2, "@").collect();
	if bitcoind_rpc_info_parts.len() != 2 {
		println!("ERROR: bad bitcoind RPC URL provided");
		return Err(());
	}
	let rpc_user_and_password: Vec<&str> = bitcoind_rpc_info_parts[1].split(":").collect();
	if rpc_user_and_password.len() != 2 {
		println!("ERROR: bad bitcoind RPC username/password combo provided");
		return Err(());
	}
	let bitcoind_rpc_username = rpc_user_and_password[0].to_string();
	let bitcoind_rpc_password = rpc_user_and_password[1].to_string();
	let bitcoind_rpc_path: Vec<&str> = bitcoind_rpc_info_parts[0].split(":").collect();
	if bitcoind_rpc_path.len() != 2 {
		println!("ERROR: bad bitcoind RPC path provided");
		return Err(());
	}
	let bitcoind_rpc_host = bitcoind_rpc_path[0].to_string();
	let bitcoind_rpc_port = bitcoind_rpc_path[1].parse::<u16>().unwrap();

	let ldk_storage_dir_path = env::args().skip(2).next().unwrap();

	let mut ldk_peer_port_set = true;
	let ldk_peer_listening_port: u16 = match env::args().skip(3).next().map(|p| p.parse()) {
		Some(Ok(p)) => p,
		Some(Err(_)) => {
			ldk_peer_port_set = false;
			9735
		}
		None => {
			ldk_peer_port_set = false;
			9735
		}
	};

	let mut arg_idx = match ldk_peer_port_set {
		true => 4,
		false => 3,
	};
	let network: Network = match env::args().skip(arg_idx).next().as_ref().map(String::as_str) {
		Some("testnet") => Network::Testnet,
		Some("regtest") => Network::Regtest,
		Some("signet") => Network::Signet,
		Some(net) => {
			panic!("Unsupported network provided. Options are: `regtest`, `testnet`, and `signet`. Got {}", net);
		}
		None => Network::Testnet,
	};

	let ldk_announced_node_name = match env::args().skip(arg_idx + 1).next().as_ref() {
		Some(s) => {
			if s.len() > 32 {
				panic!("Node Alias can not be longer than 32 bytes");
			}
			arg_idx += 1;
			let mut bytes = [0; 32];
			bytes[..s.len()].copy_from_slice(s.as_bytes());
			bytes
		}
		None => [0; 32],
	};

	let mut ldk_announced_listen_addr = Vec::new();
	loop {
		match env::args().skip(arg_idx + 1).next().as_ref() {
			Some(s) => match IpAddr::from_str(s) {
				Ok(IpAddr::V4(a)) => {
					ldk_announced_listen_addr
						.push(NetAddress::IPv4 { addr: a.octets(), port: ldk_peer_listening_port });
					arg_idx += 1;
				}
				Ok(IpAddr::V6(a)) => {
					ldk_announced_listen_addr
						.push(NetAddress::IPv6 { addr: a.octets(), port: ldk_peer_listening_port });
					arg_idx += 1;
				}
				Err(_) => panic!("Failed to parse announced-listen-addr into an IP address"),
			},
			None => break,
		}
	}

	Ok(LdkUserInfo {
		bitcoind_rpc_username,
		bitcoind_rpc_password,
		bitcoind_rpc_host,
		bitcoind_rpc_port,
		ldk_storage_dir_path,
		ldk_peer_listening_port,
		ldk_announced_listen_addr,
		ldk_announced_node_name,
		network,
	})
}
*/

async fn perform_open_channel(peer_manager: &Arc<PeerManager>, channel_manager: &Arc<ChannelManager>, ldk_data_dir: &str,
	peer_pubkey_and_ip_addr: &str, channel_value_sat: &str, announce_channel: bool) {
	let (pubkey, peer_addr) =
		match parse_peer_info(peer_pubkey_and_ip_addr.to_string()) {
			Ok(info) => info,
			Err(e) => {
				println!("{:?}", e.into_inner().unwrap());
				return;
			}
		};

	let chan_amt_sat: Result<u64, _> = channel_value_sat.parse();
	if chan_amt_sat.is_err() {
		println!("ERROR: channel amount must be a number");
		return;
	}

	if connect_peer_if_necessary(pubkey, peer_addr, peer_manager.clone())
		.await
		.is_err()
	{
		return;
	};

	println!("Opening channel (cap {} sats)...", chan_amt_sat.as_ref().unwrap());
	if open_channel(
		pubkey,
		chan_amt_sat.unwrap(),
		announce_channel,
		channel_manager.clone(),
	)
	.is_ok()
	{
		let peer_data_path = format!("{}/channel_peer_data", ldk_data_dir.clone());
		let _ = disk::persist_channel_peer(
			Path::new(&peer_data_path),
			peer_pubkey_and_ip_addr,
		);
	}
}

// Check if auto channel should/could be done, returns:
// - channel open is needed
// - channel open is possible
// - open channel amount
// - use max amount
fn auto_channel_open_check(send_amount_sat: f64, wallet: &Wallet, channel_manager: &ChannelManager) -> (bool, bool, u64, bool) {
	// Very strict minimum amount: 10% higher than send amount for lightning reserver
	let strict_min = (send_amount_sat * 1.13).ceil() as u64 + 4000;
	// optimal amount: higher than send amount, to have higher capacity
	let optimal: u64 =  strict_min + (send_amount_sat * 0.25).ceil() as u64 + 2000;
	// a bit highre to incorp. fee safely
	let fee_buffer: u64 = (send_amount_sat * 0.01).ceil() as u64 + 2000;

	// check avail. lightning balance
	let channels = &channel_manager.list_channels();
	let mut sum_balance = 0;
	for c in channels {
		sum_balance += c.balance_msat;
	}

	if sum_balance >= strict_min {
		// we are fine, enough ln balance
		return (false, false, 0, false);
	}

	// assume wallet L1 balance is up to date
	let l1_balance = (wallet.balance * 100_000_000.0).floor() as u64;
	if l1_balance > optimal + fee_buffer {
		// we should open
		return (true, true, optimal, false)
	}
	if l1_balance > strict_min + fee_buffer{
		// we should open, with all available
		return (true, true, l1_balance, true)
	}

	// not enough balance
	(true, false, optimal, false)
}

// Open channel if needed
async fn auto_channel_open(send_amount: u64, wallet: &Wallet, peer_manager: &Arc<PeerManager>, channel_manager: &Arc<ChannelManager>, ldk_data_dir: String, env: &Env) {
	let send_amount_sat = send_amount as f64 * 0.001;
	let (open_needed, open_possible, open_amount, _use_max_amount) = auto_channel_open_check(send_amount_sat, &wallet, &channel_manager);
	if !open_needed {
		// fine, no open needed
		return;
	}
	if !open_possible {
		println!("Warning: A new channel should be opened, but there is not enough balance for that. Pay {}", send_amount_sat);
		return;
	}

	// perform auto open
	let peer_pubkey_and_ip_addr = env.default_peer.as_str();
	if peer_pubkey_and_ip_addr == "" {
		println!("ERROR: default peer is unset (see .env)");
		return;
	}

	println!("AUTO OPENING channel to default peer with capacity {} (to pay: {}, l1 bal: {})", open_amount, send_amount_sat, wallet.balance * 100_000_000.0);
	let announce_channel = true; // public TODO
	perform_open_channel(&peer_manager, &channel_manager, ldk_data_dir.as_str(), &peer_pubkey_and_ip_addr, open_amount.to_string().as_str(), announce_channel).await;

	// TODO waiting a hardcoded amount, until channel is openeded
	println!("Waiting for channel opening ...");
	std::thread::sleep(std::time::Duration::from_millis(10000));
}

pub(crate) async fn poll_for_user_input<E: EventHandler>(
	invoice_payer: Arc<InvoicePayer<E>>, peer_manager: Arc<PeerManager>,
	channel_manager: Arc<ChannelManager>, 
	chain_monitor: Arc<ChainMonitor>, // needed for balances
	channelmonitors: Arc<Vec<(BlockHash, ChannelMonitor<InMemorySigner>)>>, // needed for balances
	keys_manager: Arc<KeysManager>,
	wallet: &Arc<Wallet>,
	network_graph: Arc<NetworkGraph>, inbound_payments: PaymentInfoStorage,
	outbound_payments: PaymentInfoStorage, ldk_data_dir: String, network: Network, env: &Env
) {
	println!("LDK startup successful. To view available commands: \"help\".");
	println!("LDK logs are available at <your-supplied-ldk-data-dir-path>/.ldk/logs");
	println!("Local Node ID is {}.", channel_manager.get_our_node_id());
	let stdin = io::stdin();
	let mut line_reader = stdin.lock().lines();
	loop {
		print!("> ");
		io::stdout().flush().unwrap(); // Without flushing, the `>` doesn't print
		let line = match line_reader.next() {
			Some(l) => l.unwrap(),
			None => break,
		};
		let mut words = line.split_whitespace();
		if let Some(word) = words.next() {
			match word {
				"help" => help(),

				"opendc" => {
					let peer_pubkey_and_ip_addr = env.default_peer.as_str();
					if peer_pubkey_and_ip_addr == "" {
						println!("ERROR: default peer is unset (see .env)");
						continue;
					}
					let channel_value_sat = words.next();
					let announce_channel = true; // public TODO
					if channel_value_sat.is_none() {
						println!("ERROR: opendc has 1 required argument: `opendc channel_amt_satoshis`");
						continue;
					}
					//let peer_pubkey_and_ip_addr = peer_pubkey_and_ip_addr.unwrap();

					perform_open_channel(&peer_manager, &channel_manager, ldk_data_dir.as_str(), &peer_pubkey_and_ip_addr, channel_value_sat.unwrap(), announce_channel).await;
					continue;
				}

				"pay" => {
					let invoice_str = words.next();
					if invoice_str.is_none() {
						println!("ERROR: pay requires an invoice: `pay <invoice>`");
						continue;
					}

					let invoice = match Invoice::from_str(invoice_str.unwrap()) {
						Ok(inv) => inv,
						Err(e) => {
							println!("ERROR: invalid invoice: {:?}", e);
							continue;
						}
					};
					let send_amount = invoice.amount_milli_satoshis().unwrap();

					auto_channel_open(send_amount, &wallet, &peer_manager, &channel_manager, ldk_data_dir.clone(), env).await;

					println!("Invoice with amount {} msats, paying ...", send_amount);

					send_payment(&*invoice_payer, &invoice, outbound_payments.clone());
				}

				"status" => {
					print_status_balance(&wallet, &channel_manager, &chain_monitor, &channelmonitors, true);
				}

				"openchannel" => {
					let peer_pubkey_and_ip_addr = words.next();
					let channel_value_sat = words.next();
					let announce_channel = match words.next() {
						Some("--public") | Some("--public=true") => true,
						Some("--public=false") => false,
						Some(_) => {
							println!("ERROR: invalid `--public` command format. Valid formats: `--public`, `--public=true` `--public=false`");
							continue;
						}
						None => false,
					};

					if peer_pubkey_and_ip_addr.is_none() || channel_value_sat.is_none() {
						println!("ERROR: openchannel has 2 required arguments: `openchannel pubkey@host:port channel_amt_satoshis` [--public]");
						continue;
					}
					//let peer_pubkey_and_ip_addr = peer_pubkey_and_ip_addr.unwrap();

					perform_open_channel(&peer_manager, &channel_manager, ldk_data_dir.as_str(), &peer_pubkey_and_ip_addr.unwrap(), channel_value_sat.unwrap(), announce_channel).await;
					continue;
				}

				"sendpayment" => {
					let invoice_str = words.next();
					if invoice_str.is_none() {
						println!("ERROR: sendpayment requires an invoice: `sendpayment <invoice>`");
						continue;
					}

					let invoice = match Invoice::from_str(invoice_str.unwrap()) {
						Ok(inv) => inv,
						Err(e) => {
							println!("ERROR: invalid invoice: {:?}", e);
							continue;
						}
					};
					println!("Invoice with amount {} msats, paying ...", invoice.amount_milli_satoshis().unwrap());

					send_payment(&*invoice_payer, &invoice, outbound_payments.clone());
				}

				"keysend" => {
					let dest_pubkey = match words.next() {
						Some(dest) => match hex_utils::to_compressed_pubkey(dest) {
							Some(pk) => pk,
							None => {
								println!("ERROR: couldn't parse destination pubkey");
								continue;
							}
						},
						None => {
							println!("ERROR: keysend requires a destination pubkey: `keysend <dest_pubkey> <amt_msat>`");
							continue;
						}
					};
					let amt_msat_str = match words.next() {
						Some(amt) => amt,
						None => {
							println!("ERROR: keysend requires an amount in millisatoshis: `keysend <dest_pubkey> <amt_msat>`");
							continue;
						}
					};
					let amt_msat: u64 = match amt_msat_str.parse() {
						Ok(amt) => amt,
						Err(e) => {
							println!("ERROR: couldn't parse amount_msat: {}", e);
							continue;
						}
					};
					keysend(
						&*invoice_payer,
						dest_pubkey,
						amt_msat,
						&*keys_manager,
						outbound_payments.clone(),
					);
				}
				"getinvoice" => {
					let amt_str = words.next();
					if amt_str.is_none() {
						println!("ERROR: getinvoice requires an amount in millisatoshis");
						continue;
					}

					let amt_msat: Result<u64, _> = amt_str.unwrap().parse();
					if amt_msat.is_err() {
						println!("ERROR: getinvoice provided payment amount was not a number");
						continue;
					}

					let expiry_secs_str = words.next();
					if expiry_secs_str.is_none() {
						println!("ERROR: getinvoice requires an expiry in seconds");
						continue;
					}

					let expiry_secs: Result<u32, _> = expiry_secs_str.unwrap().parse();
					if expiry_secs.is_err() {
						println!("ERROR: getinvoice provided expiry was not a number");
						continue;
					}

					get_invoice(
						amt_msat.unwrap(),
						inbound_payments.clone(),
						channel_manager.clone(),
						keys_manager.clone(),
						network,
						expiry_secs.unwrap(),
					);
				}
				"connectpeer" => {
					let peer_pubkey_and_ip_addr = words.next();
					if peer_pubkey_and_ip_addr.is_none() {
						println!("ERROR: connectpeer requires peer connection info: `connectpeer pubkey@host:port`");
						continue;
					}
					let (pubkey, peer_addr) =
						match parse_peer_info(peer_pubkey_and_ip_addr.unwrap().to_string()) {
							Ok(info) => info,
							Err(e) => {
								println!("{:?}", e.into_inner().unwrap());
								continue;
							}
						};
					if connect_peer_if_necessary(pubkey, peer_addr, peer_manager.clone())
						.await
						.is_ok()
					{
						println!("SUCCESS: connected to peer {}", pubkey);
					}
				}

				"listchannels" => list_channels(&channel_manager, &network_graph),

				"listpayments" => {
					list_payments(inbound_payments.clone(), outbound_payments.clone())
				}
				"closechannel" => {
					let channel_id_str = words.next();
					if channel_id_str.is_none() {
						println!("ERROR: closechannel requires a channel ID: `closechannel <channel_id> <peer_pubkey>`");
						continue;
					}
					let channel_id_vec = hex_utils::to_vec(channel_id_str.unwrap());
					if channel_id_vec.is_none() || channel_id_vec.as_ref().unwrap().len() != 32 {
						println!("ERROR: couldn't parse channel_id");
						continue;
					}
					let mut channel_id = [0; 32];
					channel_id.copy_from_slice(&channel_id_vec.unwrap());

					let peer_pubkey_str = words.next();
					if peer_pubkey_str.is_none() {
						println!("ERROR: closechannel requires a peer pubkey: `closechannel <channel_id> <peer_pubkey>`");
						continue;
					}
					let peer_pubkey_vec = match hex_utils::to_vec(peer_pubkey_str.unwrap()) {
						Some(peer_pubkey_vec) => peer_pubkey_vec,
						None => {
							println!("ERROR: couldn't parse peer_pubkey");
							continue;
						}
					};
					let peer_pubkey = match PublicKey::from_slice(&peer_pubkey_vec) {
						Ok(peer_pubkey) => peer_pubkey,
						Err(_) => {
							println!("ERROR: couldn't parse peer_pubkey");
							continue;
						}
					};

					close_channel(channel_id, peer_pubkey, channel_manager.clone());
				}
				"forceclosechannel" => {
					let channel_id_str = words.next();
					if channel_id_str.is_none() {
						println!("ERROR: forceclosechannel requires a channel ID: `forceclosechannel <channel_id> <peer_pubkey>`");
						continue;
					}
					let channel_id_vec = hex_utils::to_vec(channel_id_str.unwrap());
					if channel_id_vec.is_none() || channel_id_vec.as_ref().unwrap().len() != 32 {
						println!("ERROR: couldn't parse channel_id");
						continue;
					}
					let mut channel_id = [0; 32];
					channel_id.copy_from_slice(&channel_id_vec.unwrap());

					let peer_pubkey_str = words.next();
					if peer_pubkey_str.is_none() {
						println!("ERROR: forceclosechannel requires a peer pubkey: `forceclosechannel <channel_id> <peer_pubkey>`");
						continue;
					}
					let peer_pubkey_vec = match hex_utils::to_vec(peer_pubkey_str.unwrap()) {
						Some(peer_pubkey_vec) => peer_pubkey_vec,
						None => {
							println!("ERROR: couldn't parse peer_pubkey");
							continue;
						}
					};
					let peer_pubkey = match PublicKey::from_slice(&peer_pubkey_vec) {
						Ok(peer_pubkey) => peer_pubkey,
						Err(_) => {
							println!("ERROR: couldn't parse peer_pubkey");
							continue;
						}
					};

					force_close_channel(channel_id, peer_pubkey, channel_manager.clone());
				}
				"nodeinfo" => node_info(&channel_manager, &peer_manager),
				"listpeers" => list_peers(peer_manager.clone()),
				"signmessage" => {
					const MSG_STARTPOS: usize = "signmessage".len() + 1;
					if line.as_bytes().len() <= MSG_STARTPOS {
						println!("ERROR: signmsg requires a message");
						continue;
					}
					println!(
						"{:?}",
						lightning::util::message_signing::sign(
							&line.as_bytes()[MSG_STARTPOS..],
							&keys_manager.get_node_secret(Recipient::Node).unwrap()
						)
					);
				}
				_ => println!("Unknown command. See `\"help\" for available commands."),
			}
		}
	}
}

fn help() {
	println!("opendc <amt_satoshis>               // open default channel");
	println!("pay <invoice>");
	println!("status");
	println!("");
	println!("openchannel pubkey@host:port <amt_satoshis> [--public]");
	println!("sendpayment <invoice>");
	println!("keysend <dest_pubkey> <amt_msats>");
	println!("getinvoice <amt_msats> <expiry_secs>");
	println!("connectpeer pubkey@host:port");
	println!("listchannels");
	println!("listpayments");
	println!("closechannel <channel_id> <peer_pubkey>");
	println!("forceclosechannel <channel_id> <peer_pubkey>");
	println!("nodeinfo");
	println!("listpeers");
	println!("signmessage <message>");
}

fn node_info(channel_manager: &Arc<ChannelManager>, peer_manager: &Arc<PeerManager>) {
	println!("\t{{");
	println!("\t\t node_pubkey: {}", channel_manager.get_our_node_id());
	let chans = channel_manager.list_channels();
	println!("\t\t num_channels: {}", chans.len());
	println!("\t\t num_usable_channels: {}", chans.iter().filter(|c| c.is_usable).count());
	let local_balance_msat = chans.iter().map(|c| c.balance_msat).sum::<u64>();
	println!("\t\t local_balance_msat: {}", local_balance_msat);
	println!("\t\t num_peers: {}", peer_manager.get_peer_node_ids().len());
	println!("\t}},");
}

fn list_peers(peer_manager: Arc<PeerManager>) {
	println!("\t{{");
	for pubkey in peer_manager.get_peer_node_ids() {
		println!("\t\t pubkey: {}", pubkey);
	}
	println!("\t}},");
}

fn list_channels(channel_manager: &Arc<ChannelManager>, network_graph: &Arc<NetworkGraph>) {
	print!("[");
	for chan_info in channel_manager.list_channels() {
		println!("");
		println!("\t{{");
		println!("\t\tchannel_id: {},", hex_utils::hex_str(&chan_info.channel_id[..]));
		if let Some(funding_txo) = chan_info.funding_txo {
			println!("\t\tfunding_txid: {},", funding_txo.txid);
		}

		println!(
			"\t\tpeer_pubkey: {},",
			hex_utils::hex_str(&chan_info.counterparty.node_id.serialize())
		);
		if let Some(node_info) = network_graph
			.read_only()
			.nodes()
			.get(&NodeId::from_pubkey(&chan_info.counterparty.node_id))
		{
			if let Some(announcement) = &node_info.announcement_info {
				println!("\t\tpeer_alias: {}", announcement.alias);
			}
		}

		if let Some(id) = chan_info.short_channel_id {
			println!("\t\tshort_channel_id: {},", id);
		}
		println!("\t\tis_channel_ready: {},", chan_info.is_channel_ready);
		println!("\t\tchannel_value_satoshis: {},", chan_info.channel_value_satoshis);
		println!("\t\tlocal_balance_msat: {},", chan_info.balance_msat);
		if chan_info.is_usable {
			println!("\t\tavailable_balance_for_send_msat: {},", chan_info.outbound_capacity_msat);
			println!("\t\tavailable_balance_for_recv_msat: {},", chan_info.inbound_capacity_msat);
		}
		println!("\t\tchannel_can_send_payments: {},", chan_info.is_usable);
		println!("\t\tpublic: {},", chan_info.is_public);
		println!("\t}},");
	}
	println!("]");
}

fn list_payments(inbound_payments: PaymentInfoStorage, outbound_payments: PaymentInfoStorage) {
	let inbound = inbound_payments.lock().unwrap();
	let outbound = outbound_payments.lock().unwrap();
	print!("[");
	for (payment_hash, payment_info) in inbound.deref() {
		println!("");
		println!("\t{{");
		println!("\t\tamount_millisatoshis: {},", payment_info.amt_msat);
		println!("\t\tpayment_hash: {},", hex_utils::hex_str(&payment_hash.0));
		println!("\t\thtlc_direction: inbound,");
		println!(
			"\t\thtlc_status: {},",
			match payment_info.status {
				HTLCStatus::Pending => "pending",
				HTLCStatus::Succeeded => "succeeded",
				HTLCStatus::Failed => "failed",
			}
		);

		println!("\t}},");
	}

	for (payment_hash, payment_info) in outbound.deref() {
		println!("");
		println!("\t{{");
		println!("\t\tamount_millisatoshis: {},", payment_info.amt_msat);
		println!("\t\tpayment_hash: {},", hex_utils::hex_str(&payment_hash.0));
		println!("\t\thtlc_direction: outbound,");
		println!(
			"\t\thtlc_status: {},",
			match payment_info.status {
				HTLCStatus::Pending => "pending",
				HTLCStatus::Succeeded => "succeeded",
				HTLCStatus::Failed => "failed",
			}
		);

		println!("\t}},");
	}
	println!("]");
}

pub(crate) async fn connect_peer_if_necessary(
	pubkey: PublicKey, peer_addr: SocketAddr, peer_manager: Arc<PeerManager>,
) -> Result<(), ()> {
	for node_pubkey in peer_manager.get_peer_node_ids() {
		if node_pubkey == pubkey {
			return Ok(());
		}
	}
	let res = do_connect_peer(pubkey, peer_addr, peer_manager).await;
	if res.is_err() {
		println!("ERROR: failed to connect to peer");
	}
	res
}

pub(crate) async fn do_connect_peer(
	pubkey: PublicKey, peer_addr: SocketAddr, peer_manager: Arc<PeerManager>,
) -> Result<(), ()> {
	match lightning_net_tokio::connect_outbound(Arc::clone(&peer_manager), pubkey, peer_addr).await
	{
		Some(connection_closed_future) => {
			let mut connection_closed_future = Box::pin(connection_closed_future);
			loop {
				match futures::poll!(&mut connection_closed_future) {
					std::task::Poll::Ready(_) => {
						return Err(());
					}
					std::task::Poll::Pending => {}
				}
				// Avoid blocking the tokio context by sleeping a bit
				match peer_manager.get_peer_node_ids().iter().find(|id| **id == pubkey) {
					Some(_) => return Ok(()),
					None => tokio::time::sleep(Duration::from_millis(10)).await,
				}
			}
		}
		None => Err(()),
	}
}

fn open_channel(
	peer_pubkey: PublicKey, channel_amt_sat: u64, announced_channel: bool,
	channel_manager: Arc<ChannelManager>,
) -> Result<(), ()> {
	// Adam: wait for 1 confirmation only instead of default 6
	let min_confirmations = 1;

	let config = UserConfig {
		channel_handshake_limits: ChannelHandshakeLimits {

			// Adam: use zeroconf
			trust_own_funding_0conf: true,

			// lnd's max to_self_delay is 2016, so we want to be compatible.
			their_to_self_delay: 2016,
			..Default::default()
		},
		channel_handshake_config: ChannelHandshakeConfig {

			// Adam
			minimum_depth: min_confirmations,

			announced_channel,
			..Default::default()
		},
		..Default::default()
	};

	match channel_manager.create_channel(peer_pubkey, channel_amt_sat, 0, 0, Some(config)) {
		Ok(_) => {
			println!("EVENT: initiated channel with peer {}. ", peer_pubkey);
			return Ok(());
		}
		Err(e) => {
			println!("ERROR: failed to open channel: {:?}", e);
			return Err(());
		}
	}
}

fn send_payment<E: EventHandler>(
	invoice_payer: &InvoicePayer<E>, invoice: &Invoice, payment_storage: PaymentInfoStorage,
) {
	let status = match invoice_payer.pay_invoice(invoice) {
		Ok(_payment_id) => {
			let payee_pubkey = invoice.recover_payee_pub_key();
			let amt_msat = invoice.amount_milli_satoshis().unwrap();
			println!("EVENT: initiated sending {} msats to {}", amt_msat, payee_pubkey);
			print!("> ");
			HTLCStatus::Pending
		}
		Err(PaymentError::Invoice(e)) => {
			println!("ERROR: invalid invoice: {}", e);
			print!("> ");
			return;
		}
		Err(PaymentError::Routing(e)) => {
			println!("ERROR: failed to find route: {}", e.err);
			print!("> ");
			return;
		}
		Err(PaymentError::Sending(e)) => {
			println!("ERROR: failed to send payment: {:?}", e);
			print!("> ");
			HTLCStatus::Failed
		}
	};
	let payment_hash = PaymentHash(invoice.payment_hash().clone().into_inner());
	let payment_secret = Some(invoice.payment_secret().clone());

	let mut payments = payment_storage.lock().unwrap();
	payments.insert(
		payment_hash,
		PaymentInfo {
			preimage: None,
			secret: payment_secret,
			status,
			amt_msat: MillisatAmount(invoice.amount_milli_satoshis()),
		},
	);
}

fn keysend<E: EventHandler, K: KeysInterface>(
	invoice_payer: &InvoicePayer<E>, payee_pubkey: PublicKey, amt_msat: u64, keys: &K,
	payment_storage: PaymentInfoStorage,
) {
	let payment_preimage = keys.get_secure_random_bytes();

	let status = match invoice_payer.pay_pubkey(
		payee_pubkey,
		PaymentPreimage(payment_preimage),
		amt_msat,
		40,
	) {
		Ok(_payment_id) => {
			println!("EVENT: initiated sending {} msats to {}", amt_msat, payee_pubkey);
			print!("> ");
			HTLCStatus::Pending
		}
		Err(PaymentError::Invoice(e)) => {
			println!("ERROR: invalid payee: {}", e);
			print!("> ");
			return;
		}
		Err(PaymentError::Routing(e)) => {
			println!("ERROR: failed to find route: {}", e.err);
			print!("> ");
			return;
		}
		Err(PaymentError::Sending(e)) => {
			println!("ERROR: failed to send payment: {:?}", e);
			print!("> ");
			HTLCStatus::Failed
		}
	};

	let mut payments = payment_storage.lock().unwrap();
	payments.insert(
		PaymentHash(Sha256::hash(&payment_preimage).into_inner()),
		PaymentInfo {
			preimage: None,
			secret: None,
			status,
			amt_msat: MillisatAmount(Some(amt_msat)),
		},
	);
}

fn get_invoice(
	amt_msat: u64, payment_storage: PaymentInfoStorage, channel_manager: Arc<ChannelManager>,
	keys_manager: Arc<KeysManager>, network: Network, expiry_secs: u32,
) {
	let mut payments = payment_storage.lock().unwrap();
	let currency = match network {
		Network::Bitcoin => Currency::Bitcoin,
		Network::Testnet => Currency::BitcoinTestnet,
		Network::Regtest => Currency::Regtest,
		Network::Signet => Currency::Signet,
	};
	let invoice = match utils::create_invoice_from_channelmanager(
		&channel_manager,
		keys_manager,
		currency,
		Some(amt_msat),
		"ldk-tutorial-node".to_string(),
		expiry_secs,
	) {
		Ok(inv) => {
			println!("SUCCESS: generated invoice: {}", inv);
			inv
		}
		Err(e) => {
			println!("ERROR: failed to create invoice: {:?}", e);
			return;
		}
	};

	let payment_hash = PaymentHash(invoice.payment_hash().clone().into_inner());
	payments.insert(
		payment_hash,
		PaymentInfo {
			preimage: None,
			secret: Some(invoice.payment_secret().clone()),
			status: HTLCStatus::Pending,
			amt_msat: MillisatAmount(Some(amt_msat)),
		},
	);
}

fn close_channel(
	channel_id: [u8; 32], counterparty_node_id: PublicKey, channel_manager: Arc<ChannelManager>,
) {
	match channel_manager.close_channel(&channel_id, &counterparty_node_id) {
		Ok(()) => println!("EVENT: initiating channel close"),
		Err(e) => println!("ERROR: failed to close channel: {:?}", e),
	}
}

fn force_close_channel(
	channel_id: [u8; 32], counterparty_node_id: PublicKey, channel_manager: Arc<ChannelManager>,
) {
	match channel_manager.force_close_broadcasting_latest_txn(&channel_id, &counterparty_node_id) {
		Ok(()) => println!("EVENT: initiating channel force-close"),
		Err(e) => println!("ERROR: failed to force-close channel: {:?}", e),
	}
}

pub(crate) fn parse_peer_info(
	peer_pubkey_and_ip_addr: String,
) -> Result<(PublicKey, SocketAddr), std::io::Error> {
	let mut pubkey_and_addr = peer_pubkey_and_ip_addr.split("@");
	let pubkey = pubkey_and_addr.next();
	let peer_addr_str = pubkey_and_addr.next();
	if peer_addr_str.is_none() || peer_addr_str.is_none() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::Other,
			"ERROR: incorrectly formatted peer info. Should be formatted as: `pubkey@host:port`",
		));
	}

	let peer_addr = peer_addr_str.unwrap().to_socket_addrs().map(|mut r| r.next());
	if peer_addr.is_err() || peer_addr.as_ref().unwrap().is_none() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::Other,
			"ERROR: couldn't parse pubkey@host:port into a socket address",
		));
	}

	let pubkey = hex_utils::to_compressed_pubkey(pubkey.unwrap());
	if pubkey.is_none() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::Other,
			"ERROR: unable to parse given pubkey for node",
		));
	}

	Ok((pubkey.unwrap(), peer_addr.unwrap().unwrap()))
}
