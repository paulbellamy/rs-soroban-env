use crate::host::Host;
use crate::native_contract::base_types::BigInt;
use crate::native_contract::token::error::Error;
use crate::native_contract::token::public_types::Identifier;
use crate::native_contract::token::storage_types::DataKey;
use core::cmp::Ordering;
use soroban_env_common::{CheckedEnv, TryIntoVal};

pub fn read_balance(e: &Host, id: Identifier) -> Result<BigInt, Error> {
    let key = DataKey::Balance(id);
    if let Ok(balance) = e.get_contract_data(key.try_into_val(e)?) {
        Ok(balance.try_into_val(e)?)
    } else {
        Ok(BigInt::from_u64(e, 0)?)
    }
}

fn write_balance(e: &Host, id: Identifier, amount: BigInt) -> Result<(), Error> {
    let key = DataKey::Balance(id);
    e.put_contract_data(key.try_into_val(e)?, amount.try_into_val(e)?)?;
    Ok(())
}

pub fn receive_balance(e: &Host, id: Identifier, amount: BigInt) -> Result<(), Error> {
    let balance = read_balance(e, id.clone())?;
    let is_frozen = read_state(e, id.clone())?;
    if is_frozen {
        Err(Error::ContractError)
    } else {
        write_balance(e, id, (balance + amount)?)
    }
}

pub fn spend_balance(e: &Host, id: Identifier, amount: BigInt) -> Result<(), Error> {
    let balance = read_balance(e, id.clone())?;
    let is_frozen = read_state(e, id.clone())?;
    if is_frozen {
        Err(Error::ContractError)
    } else if balance.compare(&amount)? == Ordering::Less {
        Err(Error::ContractError)
    } else {
        write_balance(e, id, (balance - amount)?)
    }
}

pub fn read_state(e: &Host, id: Identifier) -> Result<bool, Error> {
    let key = DataKey::State(id);
    if let Ok(state) = e.get_contract_data(key.try_into_val(e)?) {
        Ok(state.try_into()?)
    } else {
        Ok(false)
    }
}

pub fn write_state(e: &Host, id: Identifier, is_frozen: bool) -> Result<(), Error> {
    let key = DataKey::State(id);
    e.put_contract_data(key.try_into_val(e)?, is_frozen.into())?;
    Ok(())
}
