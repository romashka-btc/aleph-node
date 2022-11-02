use codec::Decode;
use frame_support::{dispatch::Weight, log::error};
use pallet_contracts::chain_extension::{
    ChainExtension, Environment, Ext, InitState, RetVal, SysConfig,
};
use pallet_snarcos::{Config, Error, Pallet as Snarcos, VerificationKeyIdentifier, WeightInfo};
use sp_core::crypto::UncheckedFrom;
use sp_runtime::DispatchError;
use sp_std::{mem::size_of, vec::Vec};

use crate::{MaximumVerificationKeyLength, Runtime};

pub const SNARCOS_STORE_KEY_FUNC_ID: u32 = 41;

// Return codes for `pallet_snarcos::store_key`.
pub const SNARCOS_STORE_KEY_OK: u32 = 10_000;
pub const SNARCOS_STORE_KEY_TOO_LONG_KEY: u32 = 10_001;
pub const SNARCOS_STORE_KEY_IN_USE: u32 = 10_002;
pub const SNARCOS_STORE_KEY_ERROR_UNKNOWN: u32 = 10_003;

pub struct SnarcosChainExtension;

impl ChainExtension<Runtime> for SnarcosChainExtension {
    fn call<E: Ext>(func_id: u32, env: Environment<E, InitState>) -> Result<RetVal, DispatchError>
    where
        <E::T as SysConfig>::AccountId: UncheckedFrom<<E::T as SysConfig>::Hash> + AsRef<[u8]>,
    {
        match func_id {
            SNARCOS_STORE_KEY_FUNC_ID => Self::snarcos_store_key(env),
            _ => {
                error!("Called an unregistered `func_id`: {}", func_id);
                Err(DispatchError::Other("Unimplemented func_id"))
            }
        }
    }
}

pub type ByteCount = u32;

/// Struct to be decoded from a byte slice passed from the contract.
///
/// Notice, that contract can pass these arguments one by one, not necessarily as such struct. Only
/// the order of values is important.
///
/// It cannot be `MaxEncodedLen` due to `Vec<_>` and thus `Environment::read_as` cannot be used.
#[derive(Decode)]
struct StoreKeyArgs {
    pub identifier: VerificationKeyIdentifier,
    pub key: Vec<u8>,
}

impl SnarcosChainExtension {
    fn store_key_weight(key_length: ByteCount) -> Weight {
        <<Runtime as Config>::WeightInfo as WeightInfo>::store_key(key_length)
    }

    fn snarcos_store_key<E: Ext>(env: Environment<E, InitState>) -> Result<RetVal, DispatchError>
    where
        <E::T as SysConfig>::AccountId: UncheckedFrom<<E::T as SysConfig>::Hash> + AsRef<[u8]>,
    {
        // We need to read input as plain bytes (encoded args).
        let mut env = env.buf_in_buf_out();

        // Check if it makes sense to read and decode data.
        let key_length = env
            .in_len()
            .saturating_sub(size_of::<VerificationKeyIdentifier>() as ByteCount);
        if key_length > MaximumVerificationKeyLength::get() {
            return Ok(RetVal::Converging(SNARCOS_STORE_KEY_TOO_LONG_KEY));
        }

        // We charge now - even if decoding fails and we shouldn't touch storage, we have to incur
        // fee for reading memory.
        env.charge_weight(Self::store_key_weight(key_length))?;

        // Parsing will have to be done here. This is due to the fact that methods
        // `Environment<_,_,_,S: BufIn>::read*` don't move starting pointer and thus we can make
        // only a single read. Since `key` is just an ('unbounded') `Vec<u8>` we can only use
        // `env.read()` method and decode arguments by hand here.
        //
        // It is safe to read `env.in_len()` bytes since we already checked that it's not too much.
        let bytes = env.read(env.in_len())?;

        let args = StoreKeyArgs::decode(&mut &*bytes)
            .map_err(|_| DispatchError::Other("Failed to decode arguments"))?;

        // Pass the arguments to the pallet and interpret the result.
        let return_status = match Snarcos::<Runtime>::bare_store_key(args.identifier, args.key) {
            Ok(_) => SNARCOS_STORE_KEY_OK,
            // In case `DispatchResultWithPostInfo` was returned (or some simpler equivalent for
            // `bare_store_key`), we could adjust weight. However, for the storing key action it
            // doesn't make sense.
            Err(Error::<Runtime>::VerificationKeyTooLong) => SNARCOS_STORE_KEY_TOO_LONG_KEY,
            Err(Error::<Runtime>::IdentifierAlreadyInUse) => SNARCOS_STORE_KEY_IN_USE,
            _ => SNARCOS_STORE_KEY_ERROR_UNKNOWN,
        };
        Ok(RetVal::Converging(return_status))
    }
}
