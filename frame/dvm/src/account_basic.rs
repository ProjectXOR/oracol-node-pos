use crate::{Config, OxorBalance, RemainingOxorBalance, RemainingXorBalance, XorBalance};
use oracol_evm::{Account as EVMAccount, AccountBasic, AddressMapping};
use oracol_support::evm::POW_9;
use evm::ExitError;
use frame_support::ensure;
use frame_support::{storage::StorageMap, traits::Currency};
use sp_core::{H160, U256};
use sp_runtime::{
	traits::{Saturating, UniqueSaturatedInto},
	SaturatedConversion,
};

pub trait RemainBalanceOp<T: Config, B> {
	/// Get the remaining balance
	fn remaining_balance(account_id: &T::AccountId) -> B;
	/// Set the remaining balance
	fn set_remaining_balance(account_id: &T::AccountId, value: B);
	/// Remove the remaining balance
	fn remove_remaining_balance(account_id: &T::AccountId);
	/// Inc remaining balance
	fn inc_remaining_balance(account_id: &T::AccountId, value: B);
	/// Dec remaining balance
	fn dec_remaining_balance(account_id: &T::AccountId, value: B);
}

pub struct XorRemainBalance;
impl<T: Config> RemainBalanceOp<T, XorBalance<T>> for XorRemainBalance {
	/// Get the remaining balance
	fn remaining_balance(account_id: &T::AccountId) -> XorBalance<T> {
		<RemainingXorBalance<T>>::get(account_id)
	}
	/// Set the remaining balance
	fn set_remaining_balance(account_id: &T::AccountId, value: XorBalance<T>) {
		<RemainingXorBalance<T>>::insert(account_id, value)
	}
	/// Remove the remaining balance
	fn remove_remaining_balance(account_id: &T::AccountId) {
		<RemainingXorBalance<T>>::remove(account_id)
	}
	/// Inc remaining balance
	fn inc_remaining_balance(account_id: &T::AccountId, value: XorBalance<T>) {
		let remain_balance =
			<Self as RemainBalanceOp<T, XorBalance<T>>>::remaining_balance(account_id);
		let updated_balance = remain_balance.saturating_add(value);
		<RemainingXorBalance<T>>::insert(account_id, updated_balance);
	}
	/// Dec remaining balance
	fn dec_remaining_balance(account_id: &T::AccountId, value: XorBalance<T>) {
		let remain_balance =
			<Self as RemainBalanceOp<T, XorBalance<T>>>::remaining_balance(account_id);
		let updated_balance = remain_balance.saturating_sub(value);
		<RemainingXorBalance<T>>::insert(account_id, updated_balance);
	}
}

pub struct OxorRemainBalance;
impl<T: Config> RemainBalanceOp<T, OxorBalance<T>> for OxorRemainBalance {
	/// Get the remaining balance
	fn remaining_balance(account_id: &T::AccountId) -> OxorBalance<T> {
		<RemainingOxorBalance<T>>::get(account_id)
	}
	/// Set the remaining balance
	fn set_remaining_balance(account_id: &T::AccountId, value: OxorBalance<T>) {
		<RemainingOxorBalance<T>>::insert(account_id, value)
	}
	/// Remove the remaining balance
	fn remove_remaining_balance(account_id: &T::AccountId) {
		<RemainingOxorBalance<T>>::remove(account_id)
	}
	/// Inc remaining balance
	fn inc_remaining_balance(account_id: &T::AccountId, value: OxorBalance<T>) {
		let remain_balance =
			<Self as RemainBalanceOp<T, OxorBalance<T>>>::remaining_balance(account_id);
		let updated_balance = remain_balance.saturating_add(value);
		<RemainingOxorBalance<T>>::insert(account_id, updated_balance);
	}
	/// Dec remaining balance
	fn dec_remaining_balance(account_id: &T::AccountId, value: OxorBalance<T>) {
		let remain_balance =
			<Self as RemainBalanceOp<T, OxorBalance<T>>>::remaining_balance(account_id);
		let updated_balance = remain_balance.saturating_sub(value);
		<RemainingOxorBalance<T>>::insert(account_id, updated_balance);
	}
}

pub struct DvmAccountBasic<T, C, RB>(sp_std::marker::PhantomData<(T, C, RB)>);
impl<T: Config, C, RB> AccountBasic for DvmAccountBasic<T, C, RB>
where
	RB: RemainBalanceOp<T, C::Balance>,
	C: Currency<T::AccountId>,
{
	/// Get the account basic in EVM format.
	fn account_basic(address: &H160) -> EVMAccount {
		let account_id = <T as oracol_evm::Config>::AddressMapping::into_account_id(*address);
		let nonce = <frame_system::Pallet<T>>::account_nonce(&account_id);
		let helper = U256::from(POW_9);

		// Get balance from Currency
		let balance: U256 = C::free_balance(&account_id).saturated_into::<u128>().into();

		// Get remaining balance from dvm
		let remaining_balance: U256 = RB::remaining_balance(&account_id)
			.saturated_into::<u128>()
			.into();

		// Final balance = balance * 10^9 + remaining_balance
		let final_balance = (balance * helper)
			.checked_add(remaining_balance)
			.unwrap_or_default();

		EVMAccount {
			nonce: nonce.saturated_into::<u128>().into(),
			balance: final_balance,
		}
	}

	/// Mutate the basic account
	fn mutate_account_basic(address: &H160, new: EVMAccount) {
		let helper = U256::from(POW_9);

		let account_id = <T as oracol_evm::Config>::AddressMapping::into_account_id(*address);
		let current = Self::account_basic(address);
		let dvm_balance: U256 = RB::remaining_balance(&account_id)
			.saturated_into::<u128>()
			.into();

		let nb = new.balance;
		match current.balance {
			cb if cb > nb => {
				let diff = cb - nb;
				let (diff_balance, diff_remaining_balance) = diff.div_mod(helper);
				// If the dvm storage < diff remaining balance, we can not do sub operation directly.
				// Otherwise, slash Currency, dec dvm storage balance directly.
				if dvm_balance < diff_remaining_balance {
					let remaining_balance = dvm_balance
						.saturating_add(U256::from(1) * helper)
						.saturating_sub(diff_remaining_balance);

					C::slash(
						&account_id,
						(diff_balance + 1).low_u128().unique_saturated_into(),
					);
					RB::set_remaining_balance(
						&account_id,
						remaining_balance.low_u128().saturated_into(),
					);
				} else {
					C::slash(&account_id, diff_balance.low_u128().unique_saturated_into());
					RB::dec_remaining_balance(
						&account_id,
						diff_remaining_balance.low_u128().saturated_into(),
					);
				}
			}
			cb if cb < nb => {
				let diff = nb - cb;
				let (diff_balance, diff_remaining_balance) = diff.div_mod(helper);

				// If dvm storage balance + diff remaining balance > helper, we must update Currency balance.
				if dvm_balance + diff_remaining_balance >= helper {
					let remaining_balance = dvm_balance + diff_remaining_balance - helper;

					C::deposit_creating(
						&account_id,
						(diff_balance + 1).low_u128().unique_saturated_into(),
					);
					RB::set_remaining_balance(
						&account_id,
						remaining_balance.low_u128().saturated_into(),
					);
				} else {
					C::deposit_creating(
						&account_id,
						diff_balance.low_u128().unique_saturated_into(),
					);
					RB::inc_remaining_balance(
						&account_id,
						diff_remaining_balance.low_u128().saturated_into(),
					);
				}
			}
			_ => return,
		}

		// Handle existential deposit
		let xor_existential_deposit: u128 =
			<T as Config>::XorCurrency::minimum_balance().saturated_into::<u128>();
		let oxor_existential_deposit: u128 =
			<T as Config>::OxorCurrency::minimum_balance().saturated_into::<u128>();
		let xor_existential_deposit = U256::from(xor_existential_deposit) * helper;
		let oxor_existential_deposit = U256::from(oxor_existential_deposit) * helper;

		let xor_account = T::XorAccountBasic::account_basic(address);
		let oxor_account = T::OxorAccountBasic::account_basic(address);
		if xor_account.balance < xor_existential_deposit
			&& oxor_account.balance < oxor_existential_deposit
		{
			<XorRemainBalance as RemainBalanceOp<T, XorBalance<T>>>::remove_remaining_balance(
				&account_id,
			);
			<OxorRemainBalance as RemainBalanceOp<T, OxorBalance<T>>>::remove_remaining_balance(
				&account_id,
			);
		}
	}

	fn transfer(source: &H160, target: &H160, value: U256) -> Result<(), ExitError> {
		let source_account = Self::account_basic(source);
		ensure!(source_account.balance >= value, ExitError::OutOfGas);
		let new_source_balance = source_account.balance.saturating_sub(value);
		Self::mutate_account_basic(
			source,
			EVMAccount {
				nonce: source_account.nonce,
				balance: new_source_balance,
			},
		);

		let target_account = Self::account_basic(target);
		let new_target_balance = target_account.balance.saturating_add(value);
		Self::mutate_account_basic(
			target,
			EVMAccount {
				nonce: target_account.nonce,
				balance: new_target_balance,
			},
		);

		Ok(())
	}
}