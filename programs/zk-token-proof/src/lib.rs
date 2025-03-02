#![forbid(unsafe_code)]

use {
    bytemuck::Pod,
    solana_program_runtime::{ic_msg, invoke_context::InvokeContext},
    solana_sdk::{
        instruction::{InstructionError, TRANSACTION_LEVEL_STACK_HEIGHT},
        system_program,
    },
    solana_zk_token_sdk::{
        zk_token_proof_instruction::*,
        zk_token_proof_program::id,
        zk_token_proof_state::{ProofContextState, ProofContextStateMeta},
    },
    std::result::Result,
};

fn process_verify_proof<T, U>(invoke_context: &mut InvokeContext) -> Result<(), InstructionError>
where
    T: Pod + ZkProofData<U>,
    U: Pod,
{
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let instruction_data = instruction_context.get_instruction_data();
    let proof_data = ProofInstruction::proof_data::<T, U>(instruction_data).ok_or_else(|| {
        ic_msg!(invoke_context, "invalid proof data");
        InstructionError::InvalidInstructionData
    })?;

    proof_data.verify_proof().map_err(|err| {
        ic_msg!(invoke_context, "proof_verification failed: {:?}", err);
        InstructionError::InvalidInstructionData
    })?;

    // create context state if accounts are provided with the instruction
    if instruction_context.get_number_of_instruction_accounts() > 0 {
        let context_state_authority = *instruction_context
            .try_borrow_instruction_account(transaction_context, 1)?
            .get_key();

        let mut proof_context_account =
            instruction_context.try_borrow_instruction_account(transaction_context, 0)?;

        if *proof_context_account.get_owner() != id() {
            return Err(InstructionError::InvalidAccountOwner);
        }

        let proof_context_state_meta =
            ProofContextStateMeta::try_from_bytes(proof_context_account.get_data())?;

        if proof_context_state_meta.proof_type != ProofType::Uninitialized.into() {
            return Err(InstructionError::AccountAlreadyInitialized);
        }

        let context_state_data = ProofContextState::encode(
            &context_state_authority,
            T::PROOF_TYPE,
            proof_data.context_data(),
        );

        if proof_context_account.get_data().len() != context_state_data.len() {
            return Err(InstructionError::InvalidAccountData);
        }

        proof_context_account.set_data(context_state_data)?;
    }

    Ok(())
}

fn process_close_proof_context(invoke_context: &mut InvokeContext) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;

    let owner_pubkey = {
        let owner_account =
            instruction_context.try_borrow_instruction_account(transaction_context, 2)?;

        if !owner_account.is_signer() {
            return Err(InstructionError::MissingRequiredSignature);
        }
        *owner_account.get_key()
    }; // done with `owner_account`, so drop it to prevent a potential double borrow

    let proof_context_account_pubkey = *instruction_context
        .try_borrow_instruction_account(transaction_context, 0)?
        .get_key();
    let destination_account_pubkey = *instruction_context
        .try_borrow_instruction_account(transaction_context, 1)?
        .get_key();
    if proof_context_account_pubkey == destination_account_pubkey {
        return Err(InstructionError::InvalidInstructionData);
    }

    let mut proof_context_account =
        instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let proof_context_state_meta =
        ProofContextStateMeta::try_from_bytes(proof_context_account.get_data())?;
    let expected_owner_pubkey = proof_context_state_meta.context_state_authority;

    if owner_pubkey != expected_owner_pubkey {
        return Err(InstructionError::InvalidAccountOwner);
    }

    let mut destination_account =
        instruction_context.try_borrow_instruction_account(transaction_context, 1)?;
    destination_account.checked_add_lamports(proof_context_account.get_lamports())?;
    proof_context_account.set_lamports(0)?;
    proof_context_account.set_data_length(0)?;
    proof_context_account.set_owner(system_program::id().as_ref())?;

    Ok(())
}

pub fn process_instruction(invoke_context: &mut InvokeContext) -> Result<(), InstructionError> {
    if invoke_context.get_stack_height() != TRANSACTION_LEVEL_STACK_HEIGHT {
        // Not supported as an inner instruction
        return Err(InstructionError::UnsupportedProgramId);
    }

    // Consume compute units since proof verification is an expensive operation
    {
        // TODO: Tune the number of units consumed.  The current value is just a rough estimate
        invoke_context.consume_checked(100_000)?;
    }

    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let instruction_data = instruction_context.get_instruction_data();
    let instruction = ProofInstruction::instruction_type(instruction_data)
        .ok_or(InstructionError::InvalidInstructionData)?;

    match instruction {
        ProofInstruction::CloseContextState => {
            ic_msg!(invoke_context, "CloseContextState");
            process_close_proof_context(invoke_context)
        }
        ProofInstruction::VerifyCloseAccount => {
            ic_msg!(invoke_context, "VerifyCloseAccount");
            process_verify_proof::<CloseAccountData, CloseAccountProofContext>(invoke_context)
        }
        ProofInstruction::VerifyWithdraw => {
            ic_msg!(invoke_context, "VerifyWithdraw");
            process_verify_proof::<WithdrawData, WithdrawProofContext>(invoke_context)
        }
        ProofInstruction::VerifyWithdrawWithheldTokens => {
            ic_msg!(invoke_context, "VerifyWithdrawWithheldTokens");
            process_verify_proof::<WithdrawWithheldTokensData, WithdrawWithheldTokensProofContext>(
                invoke_context,
            )
        }
        ProofInstruction::VerifyTransfer => {
            ic_msg!(invoke_context, "VerifyTransfer");
            process_verify_proof::<TransferData, TransferProofContext>(invoke_context)
        }
        ProofInstruction::VerifyTransferWithFee => {
            ic_msg!(invoke_context, "VerifyTransferWithFee");
            process_verify_proof::<TransferWithFeeData, TransferWithFeeProofContext>(invoke_context)
        }
        ProofInstruction::VerifyPubkeyValidity => {
            ic_msg!(invoke_context, "VerifyPubkeyValidity");
            process_verify_proof::<PubkeyValidityData, PubkeyValidityProofContext>(invoke_context)
        }
    }
}
