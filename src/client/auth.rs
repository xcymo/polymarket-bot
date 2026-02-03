//! Authentication and signing for Polymarket API
//!
//! Implements EIP-712 typed data signing for order authentication.
//! Supports both Level 1 (EIP-712) and Level 2 (HMAC) authentication.

use crate::error::{BotError, Result};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, H256, U256};
use ethers::utils::keccak256;

// EIP-712 domain constants for CLOB auth
const CLOB_DOMAIN_NAME: &str = "ClobAuthDomain";
const CLOB_VERSION: &str = "1";
const CLOB_AUTH_MESSAGE: &str = "This message attests that I control the given wallet";

/// Signer for Polymarket API authentication
#[derive(Clone)]
pub struct PolySigner {
    wallet: LocalWallet,
    chain_id: u64,
}

impl PolySigner {
    /// Create a new signer from a private key (hex string, with or without 0x prefix)
    pub fn from_private_key(private_key: &str, chain_id: u64) -> Result<Self> {
        let key_hex = private_key.trim_start_matches("0x");
        let wallet: LocalWallet = key_hex
            .parse()
            .map_err(|e| BotError::Auth(format!("Invalid private key: {}", e)))?;
        
        let wallet = wallet.with_chain_id(chain_id);

        Ok(Self { wallet, chain_id })
    }

    /// Get the signer's address
    pub fn address(&self) -> Address {
        self.wallet.address()
    }

    /// Get the signer's address as hex string
    pub fn address_hex(&self) -> String {
        format!("{:?}", self.wallet.address())
    }
    
    /// Get chain ID
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Sign a message hash
    pub async fn sign_hash(&self, hash: H256) -> Result<String> {
        let signature = self
            .wallet
            .sign_hash(hash)
            .map_err(|e| BotError::Auth(format!("Signing failed: {}", e)))?;

        Ok(format!("0x{}", hex::encode(signature.to_vec())))
    }

    /// Sign arbitrary message (eth_sign style)
    pub async fn sign_message(&self, message: &[u8]) -> Result<String> {
        let signature = self
            .wallet
            .sign_message(message)
            .await
            .map_err(|e| BotError::Auth(format!("Signing failed: {}", e)))?;

        Ok(format!("0x{}", hex::encode(signature.to_vec())))
    }
    
    /// Sign EIP-712 ClobAuth message for Level 1 authentication
    /// This is used to create/derive API keys
    pub async fn sign_clob_auth(&self, timestamp: i64, nonce: u64) -> Result<String> {
        // Build EIP-712 domain separator
        let domain_type_hash = keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId)"
        );
        let name_hash = keccak256(CLOB_DOMAIN_NAME.as_bytes());
        let version_hash = keccak256(CLOB_VERSION.as_bytes());
        
        let mut domain_data = Vec::new();
        domain_data.extend_from_slice(&domain_type_hash);
        domain_data.extend_from_slice(&name_hash);
        domain_data.extend_from_slice(&version_hash);
        domain_data.extend_from_slice(&u256_to_bytes32(U256::from(self.chain_id)));
        let domain_separator = keccak256(&domain_data);
        
        // Build ClobAuth struct hash
        // ClobAuth(address address,string timestamp,uint256 nonce,string message)
        let struct_type_hash = keccak256(
            b"ClobAuth(address address,string timestamp,uint256 nonce,string message)"
        );
        let timestamp_hash = keccak256(timestamp.to_string().as_bytes());
        let message_hash = keccak256(CLOB_AUTH_MESSAGE.as_bytes());
        
        let mut struct_data = Vec::new();
        struct_data.extend_from_slice(&struct_type_hash);
        struct_data.extend_from_slice(&address_to_bytes32(self.wallet.address()));
        struct_data.extend_from_slice(&timestamp_hash);
        struct_data.extend_from_slice(&u256_to_bytes32(U256::from(nonce)));
        struct_data.extend_from_slice(&message_hash);
        let struct_hash = keccak256(&struct_data);
        
        // Compute final digest: keccak256("\x19\x01" + domain_separator + struct_hash)
        let mut digest_data = vec![0x19, 0x01];
        digest_data.extend_from_slice(&domain_separator);
        digest_data.extend_from_slice(&struct_hash);
        let digest = H256::from(keccak256(&digest_data));
        
        self.sign_hash(digest).await
    }

    /// Derive API key from signature (used internally by server, but we can compute locally)
    fn derive_api_key(&self, signature: &str) -> String {
        let hash = keccak256(signature.as_bytes());
        hex::encode(&hash[..16])
    }

    /// Sign an order for submission to CLOB
    pub async fn sign_order(&self, order_data: &OrderSignData) -> Result<String> {
        let domain_separator = self.compute_domain_separator();
        let struct_hash = self.compute_order_struct_hash(order_data);

        let mut data = vec![0x19, 0x01];
        data.extend_from_slice(&domain_separator);
        data.extend_from_slice(&struct_hash);
        let digest = H256::from(keccak256(&data));

        self.sign_hash(digest).await
    }

    fn compute_domain_separator(&self) -> [u8; 32] {
        let type_hash = keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );
        let name_hash = keccak256(b"Polymarket CTF Exchange");
        let version_hash = keccak256(b"1");

        let mut data = Vec::new();
        data.extend_from_slice(&type_hash);
        data.extend_from_slice(&name_hash);
        data.extend_from_slice(&version_hash);
        data.extend_from_slice(&u256_to_bytes32(U256::from(self.chain_id)));
        // Exchange contract address
        let exchange: Address = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E"
            .parse()
            .unwrap();
        data.extend_from_slice(&address_to_bytes32(exchange));

        keccak256(&data)
    }

    fn compute_order_struct_hash(&self, order: &OrderSignData) -> [u8; 32] {
        let type_hash = keccak256(
            b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)",
        );

        let mut data = Vec::new();
        data.extend_from_slice(&type_hash);
        data.extend_from_slice(&u256_to_bytes32(order.salt));
        data.extend_from_slice(&address_to_bytes32(order.maker));
        data.extend_from_slice(&address_to_bytes32(order.signer));
        data.extend_from_slice(&address_to_bytes32(order.taker));
        data.extend_from_slice(&u256_to_bytes32(order.token_id));
        data.extend_from_slice(&u256_to_bytes32(order.maker_amount));
        data.extend_from_slice(&u256_to_bytes32(order.taker_amount));
        data.extend_from_slice(&u256_to_bytes32(order.expiration));
        data.extend_from_slice(&u256_to_bytes32(order.nonce));
        data.extend_from_slice(&u256_to_bytes32(order.fee_rate_bps));
        data.extend_from_slice(&[0u8; 31]);
        data.push(order.side);
        data.extend_from_slice(&[0u8; 31]);
        data.push(order.signature_type);

        keccak256(&data)
    }
}

fn u256_to_bytes32(value: U256) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    value.to_big_endian(&mut bytes);
    bytes
}

fn address_to_bytes32(addr: Address) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(addr.as_bytes());
    bytes
}

/// API credentials for CLOB authentication
#[derive(Debug, Clone)]
pub struct ApiCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
    pub timestamp: u64,
}

/// Order data for signing
#[derive(Debug, Clone)]
pub struct OrderSignData {
    pub salt: U256,
    pub maker: Address,
    pub signer: Address,
    pub taker: Address,
    pub token_id: U256,
    pub maker_amount: U256,
    pub taker_amount: U256,
    pub expiration: U256,
    pub nonce: U256,
    pub fee_rate_bps: U256,
    pub side: u8,
    pub signature_type: u8,
}
