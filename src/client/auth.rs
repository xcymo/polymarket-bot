//! Authentication and signing for Polymarket API
//!
//! Implements EIP-712 typed data signing for order authentication.

use crate::error::{BotError, Result};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, H256, U256};
use ethers::utils::keccak256;

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

    /// Create API credentials for Polymarket CLOB
    pub async fn create_api_credentials(&self, nonce: u64) -> Result<ApiCredentials> {
        let timestamp = chrono::Utc::now().timestamp() as u64;
        let message = format!(
            "Sign in to Polymarket\n\nNonce: {}\nTimestamp: {}",
            nonce, timestamp
        );

        let signature = self.sign_message(message.as_bytes()).await?;

        Ok(ApiCredentials {
            api_key: self.derive_api_key(&signature),
            api_secret: signature,
            api_passphrase: self.address_hex(),
            timestamp,
        })
    }

    /// Derive API key from signature
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
