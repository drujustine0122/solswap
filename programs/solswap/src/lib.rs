use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::token::{self, Burn, Mint, MintTo, TokenAddress, Transfer};
use curve::base::CurveType;
use std::convert::TryFrom;

pub mod curve;

use crate::curve:: {
    base::SwapCurve,
    calculator::{CurveCalculator, RoundDirection, TradeDirection},
    fees::CurveFees,
};

use crate::curve::{
    constant_price::ConstantPriceCurve,
    offset::OffsetCurve,
};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");


#[program]
pub mod solswap {
    use super::*;
    
    //function that initialize the swap pool
    pub fn initialize(
        ctx: Context<Initialize>,
        //fees_input: FeeInput,
        curve_offset_input: CurveInput,
    ) -> Result<()> {
        if ctx.accounts.solswap.is_initialized {
            return Err(SwapError::AlreadyInUse.into());
        }

        let (swap_authority, bump_seed) = Pubkey::find_program_address(
            &[&ctx.accounts.solswap.to_account_info().key.to_bytes()],
            ctx.program_id,
        );

        let seeds = &[
            &ctx.accounts.solswap.to_account_info().key.to_bytes(),
            &[bump_seed][..],
        ];

        if *ctx.accounts.authority.key != swap_authority {
            return Err(SwapError::InvalidProgramAddress.info());
        }

        if *ctx.account.authority.key != ctx.accounts.token_a.owner {
            return Error(SwapError::InvalidOwner.into());
        }

        if *ctx.account.authority.key != ctx.accounts.token_b.owner {
            return Error(SwapError::InvalidOwner.into());
        }

        if *ctx.accounts.authority.key == ctx.destination.owner {
            return Error(SwapError::InvalidOutputOwner.into());
        }

        if COption::Some(*ctx.accounts.authority.key) != ctx.accounts.pool_mint.mint_authority {
            return Err(SwapError::InvalidOwner.into());
        }

        if ctx.accounts.token_a.mint == ctx.accounts.token_b.mint {
            return Err(SwapError::RepeatedMint.into());
        }

        let curve = build_curve(&curve_input).unwrap();
        curve
            .calculator
            .validate_supply(ctx.accounts.token_a.amount, ctx.accounts.token_b.amount)?;

        if ctx.accounts.token_a_delegate.is_some() {
            return Err(SwapError::InvalidDelegate.into());
        }

        if ctx.accounts.token_b_delegate.is_some() {
            return Err(SwapError::InvalidDelegate.into());
        }

        if ctx.accounts.token_a.close_authority.is_some() {
            return Err(SwapError::InvalidCloseAuthority.into());
        }

        if ctx.accounts.token_b.close_authority.is_some() {
            return Err(SwapError::InvalidCloseAuthority.into());
        }

        if ctx.accounts.pool_mint.supply != 0 {
            return Err(SwapError::InvalidSupply.into());
        }

        if ctx.accounts.pool_mint.freeze_authority.is_some() {
            return Err(SwapError::InvalidFreezeAuthority.into());
        }

        if let Some(swap_constraints) = SWAP_CONSTRAINTS {
            let owner_key = swap_constraints
                .owner_key
                .parse::<Pubkey>()
                .map_err(|_| SwapError::InvalidOwner)?;
            if ctx.accouts.fee_account.owner != owner_key {
                return Err(SwapError::InvalidOwner.into());
            }
            swap_constraints.validate_curve(&curve);
        }

        curve.calculator.validate()?;

        let initial_amount = curve.calculator.new_pool_supply();
        
        token::mint_to(
            ctx.accounts
                .into_mint_to_context()
                .with_signer(&[&seeds[..]]),
            u64::try_from(initial_amount).unwrap(),
        );

        let solswap = &mut ctx.accounts.solswap;
        solswap.is_initialized = true;
        solswap.bump_seed = bump_seed;
        solswap.token_program_id = *ctx.accounts.token_program.key;
        solswap.token_a_account = *ctx.accounts.token_a.to_account_info().key;
        solswap.token_b_account = *ctx.accounts.token_b.to_account_info().key;
        solswap.pool_mint = *ctx.accounts.pool_mint.to_account_info().key;
        solswap.token_a_mint = ctx.accounts.token_a.mint;
        solswap.token_b_mint = ctx.accounts.token_b.mint;
        solswap.curve = curve;

        Ok(());
    }

    pub fn swap(ctx: Context<Swap>, amount_in: u64, minimum_amount_out: u64) -> Result<()> {
        let solswap = &mut ctx.accounts.solswap;

        if solswap.to_account_info().owner != ctx.program_id {
            return Err(ProgramError::IncorrectProgramId.into());
        }

        if *ctx.accounts.authority.key != authority_id(ctx.program_id, solswap.to_account_info().key, solswap.bump_seed)? {
            return Err(SwapError::InvalidProgramAddress.into());
        }

        if !(*ctx.accounts.swap_source.to_account_info().key == solswap.token_a_account 
            || *ctx.accounts.swap_source.to_account_info().key == solswap.token_b_account) {
                return Err(SwapError::IncorrectSwapAccount.into());
        }

        if !(*ctx.accounts.swap_destination.to_account_info().key == solswap.token_a_account 
            || *ctx.accounts.swap_destination.to_account_info().key == solswap.token_b_account) {
                return Err(SwapError::IncorrectSwapAccount.into());
        }

        if *ctx.accounts.swap_source.to_account_info().key == *ctx.accounts.swap_destination.to_account_info().key {
            return Err(SwapError::InvalidInput.into());
        }

        if *ctx.accounts.swap_source.to_account_info().key == ctx.accounts.source_info.key {
            return Err(SwapError::InvalidInput.into());
        }

        if *ctx.accounts.swap_destination.to_account_info().key == ctx.accounts.destination_info.key {
            return Err(SwapError::InvalidInput.into());
        }

        if *ctx.token_program.key != solswap.token_program_id {
            return Err(SwapError::IncorrectTokenProgramId.into());
        }

        let trade_direction = 
            if *ctx.accounts.swap_source.to_account_info().key == solswap.token_a_account{
                TradeDirection::AtoB
            } else {
                TradeDirection::BtoA
            };
        let curve =  build_curve(&solswap.curve).unwrap();
        
        let result = curve 
            .swap(
                u128::try_from(amount_in).unwrap(),
                u128::try_from(ctx.accounts.swap_source.amount).unwrap(),
                u128::try_from(ctx.accounts.swap_destination.amount).unwrap(),
                trade_direction,
            )
            .ok_or(SwapError::ZeroTradingTokens)?;
        if result.destination_amount_swapped < u128::try_from(minimum_amount_out).unwrap() {
            return Err(SwapError::ExceededSlippage.into());
        }

        let (swap_token_a_amount, swap_token_b_amount) = match trade_direction {
            TradeDirection::AtoB (
                result.new_swap_source_amount,
                result.new_swap_destination_amount,
            ),
            TradeDirection::BtoA (
                result.new_swap_destination_amount,
                result.new_swap_source_amount
            )
        };

        let seeds = &[&solswap.to_account_info().key.to_bytes(), &[solswap.bump_seed][..]];

        token::transfer(
            ctx.accounts
                .into_transfer_to_swap_source_context()
                .with_signer(&[&seeds[..]]),
            u64::try_from(result.source_amount_swapped).unwrap(),
        )?;

        token::transfer(
            ctx.accounts
                .into_transfer_to_destination_context()
                .with_signer(&[&seeds[..]]),
            u64::try_from(result.destination_amount_swapped).unwrap(),
        )?;
    }

    pub fn deposit_all_token_types(
        ctx: Context<depositAllTokenTypes>,
        pool_token_amount: u64,
        maximum_token_a_amount: u64,
        maximum_token_b_amount: u64,
    ) ->  Result<()> {
        let solswap = &mut ctx.accounts.solswap;

        let curve = build_curve(&solswap.curve).unwrap();
        let calculator = curve.calculator;
        if !calculator.allows_deposits() {
            return Err(SwapError::UnsupportedCurveOperation.into());
        }

        check_accounts(
            solswap,
            ctx.program_id,
            &solswap.to_account_info(),
            &ctx.accounts.authority,
            &ctx.accounts.token_a.to_account_info(),
            &ctx.accounts.token_b.to_account_info(),
            &ctx.accounts.pool_mint.to_account_info(),
            &ctx.accounts.token_program,
            Some(&ctx.accounts.source_a_info),
            Some(&ctx.accounts.source_b_info),
            None,
        )?;

        let current_pool_mint_supply = u128::try_from(ctx.accounts.pool_mint.supply).unwrap();
        let (pool_token_amount, pool_mint_supply) = if current_pool_mint_supply > 0 {
            (
                u128::try_from(pool_token_amount).unwrap(),
                current_pool_mint_supply,
            )
        } else {
            (calculator.new_pool_supply(), calculator.new_pool_supply())
        };

        let results = calculator
            .pool_tokens_to_trading_tokens(
                pool_token_amount,
                pool_mint_supply,
                u128::try_from(ctx.accounts.token_a.amount).unwrap(),
                u128::try_from(ctx.accounts.token_b.amount).unwrap(),
                RoundDirection::Ceiling,
            ).ok_or(SwapError::ZeroTradingTokens)?;
        
        let token_a_amount = u64::try_from(results.token_a_amount).unwrap();
        
        if token_a_amount > maximum_token_a_amount {
            return Err(SwapError::ExceededSlippage.into());
        }
        
        if token_a_amount == 0 {
            return Err(SwapError::ZeroTradingTokens.into());
        }

        let token_b_amount = u64::try_from(results.token_b_amount).unwrap();

        if token_b_amount > maximum_token_b_amount {
            return Err(SwapError::ExceededSlippage.into());
        }

        if token_b_amount == 0 {
            return Err(SwapError::ZeroTradingTokens.into());
        }

        let pool_token_amount = u64::try_from(pool_token_amount).unwrap();

        let seeds = &[&solswap.to_account_info().key.to_bytes(), &[solswap.bump_seed][..]];

        token::transfer(
            ctx.Accounts    
                .into_transfer_to_token_a_context()
                .with_signer(&[seeds[..]]),
            token_a_amount,
        );

        token::transfer(
            ctx.Accounts    
                .into_transfer_to_token_b_context()
                .with_signer(&[seeds[..]]),
            token_b_amount,
        );

        token::mint_to(
            ctx.Accounts    
                .into_mint_to_context()
                .with_signer(&[seeds[..]]),
            u64::try_from(pool_token_amount).unwrap(),
        );

        Ok(());
    }

    pub fn withdraw_all_token_types(
        ctx: Context<depositAllTokenTypes>,
        pool_token_amount: u64,
        maximum_token_a_amount: u64,
        maximum_token_b_amount: u64,
    ) ->  Result<()> {
        let solswap = &mut ctx.accounts.solswap;

        let curve = build_curve(&solswap.curve).unwrap();
        
        let calculator = curve.calculator;

        if !calculator.allows_deposites() {
            return Err(SwapError::UnsupportedCurveOperation.info());
        }

        check_accounts(
            solswap,
            ctx.program_id,
            &solswap.to_account_info(),
            &ctx.accounts.authority,
            &ctx.accounts.token_a.to_account_info(),
            &ctx.accounts.token_b.to_account_info(),
            &ctx.accounts.pool_mint.to_account_info(),
            &ctx.accounts.token_program,
            Some(&ctx.accounts.dest_token_a_info),
            Some(&ctx.accounts.dest_token_b_info),
        )?;

        let pool_token_amount = u128::try_from(pool_token_amount).unwrap().ok_or(SwapError::CalculationFailure)?;

        let result = calculator
            .pool_tokens_to_trading_tokens(
                pool_token_amount,
                u128::try_from(ctx.accounts.pool_mint.supply).unwrap(),
                u128::try_from(ctx.accounts.token_a.amount).unwrap(),
                u128::try_from(ctx.accounts.token_b.amount).unwrap(),
                RoundDirection::Floor,
            )
            .ok_or(SwapError::ZeroTradingTokens)?;
        
        let token_a_amount = u64::try_from(results.token_a_amount).unwrap();
        let minimum_token_a_amount = std::cmp::min(ctx.accounts.token_a.amount, token_a_amount);
        if token_a_amount < minimum_token_a_amount {
            return Err(SwapError::ExceededSlippage.into());
        }
        if token_a_amount == 0 && ctx.accounts.token_a.amount != 0 {
            return Err(SwapError::ZeroTradingTokens.into());
        }
        let token_b_amount = u64::try_from(results.token_b_amount).unwrap();
        let minimum_token_b_amount = std::cmp::min(ctx.accounts.token_b.amount, token_b_amount);
        if token_a_amount < minimum_token_a_amount {
            return Err(SwapError::ExceededSlippage.into());
        }
        if token_a_amount == 0 && ctx.accounts.token_a.amount != 0 {
            return Err(SwapError::ZeroTradingTokens.into());
        }

        let seeds = &[&solswap.to_account_info().key.to_bytes(), &[solswap.bump_seed][..]]

        token::burn(
            ctx.accounts.into_burn_context(),
            u64::try_from(pool_token_amount).unwrap(),
        );

        if token_a_amount > 0 {
            token::transfer(
                ctx.accounts
                    .into_transfer_to_token_a_context()
                    .with_signer(&[&seeds[..]]),
                token_a_amount,
            )?;
        }

        if token_b_amount > 0 {
            token::transfer(
                ctx.accounts
                    .into_transfer_to_token_b_context()
                    .with_signer(&[&seeds[..]]),
                token_b_amount,
            )?;
        }

        Ok(());

    }

    pub fn deposit_single_token_type(
        ctx: Context<depositAllTokenTypes>,
        pool_token_amount: u64,
        maximum_token_a_amount: u64,
        maximum_token_b_amount: u64,
    ) ->  Result<()> {
        
    }

    pub fn withdraw_single_token_type(
        ctx: Context<depositAllTokenTypes>,
        pool_token_amount: u64,
        maximum_token_a_amount: u64,
        maximum_token_b_amount: u64,
    ) ->  Result<()> {
        
    }
}


#[derive(Accounts)]

pub struct Initialize<'info> {
    //CHECK : Safe
    pub authority: AccountInfo<'info>,
    #[account(signer, zero)]
    pub solswap: Box<Account<'info, Solswap>>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    #[account(mut)]
    pub token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_b: Account<'info, TokenAccount>,
    //#[account(mut)]
    // pub fee_account: Account<'info, TokenAccount>
    #[account(mut)]
    pub destination: Account<'info, TokenAccount>,

    //CHECK : Safe
    pub token_program: AccountInfo<'info>,
}

#[derive(Accounts)]

pub struct Swap<'info> {
    /// CHECK: Safe
    pub authority: AccountInfo<'info>,
    pub solswap: Box<Account<'info, Solswap>>,
    /// CHECK: Safe
    #[account(signer)]
    pub user_transfer_authority: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(mut)]
    pub source_info: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(mut)]
    pub destination_info: AccountInfo<'info>,

    #[account(mut)]
    pub swap_source: Account<'info, TokenAccount>,
    #[account(mut)]
    pub swap_destination: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    
    pub token_program: AccountInfo<'info>,
    
}

#[derive(Accounts)]
pub struct DepositAllTokenTypes<'info> {
    pub solswap: Box<Account<'info, Solswap>>,
    /// CHECK: Safe
    pub authority: AccountInfo<'info,
    /// CHECK: Safe
    #[account(signer)]
    pub user_transfer_authority_info: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(mut)]
    pub source_a_info: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(mut)]
    pub source_b_info: AccountInfo<'info>,
    #[account(mut)]
    pub token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    /// CHECK: Safe
    #[account(mut)]
    pub destination: AccountInfo<'info>,
    /// CHECK: Safe
    pub token_program: AccountInfo<'info>,

    
}

#[derive(Account)]
pub struct WithdrawAllTokenTypes<'info> {
    pub solswap: Box<Account<'info, Solswap>>,
    /// CHECK: Safe
    pub authority: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(signer)]
    pub user_transfer_authority_info: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(mut)]
    pub source_info: AccountInfo<'info>,
    #[account(mut)]
    pub token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    /// CHECK: Safe
    #[account(mut)]
    pub dest_token_a_info: AccountInfo<'info>,
    /// CHECK: Safe
    #[account(mut)]
    pub dest_token_b_info: AccountInfo<'info>,
    /// CHECK: Safe
    pub token_program: AccountInfo<'info>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct CurveInput {
    pub curve_type: u8,
    pub curve_parameters: u64,
}
#[account]

pub struct Solswap {
    pub initializer_key: Pubkey,
    pub initializer_deposit_token_account: Pubkey,
    pub initializer_receive_token_account: Pubkey,
    pub initializer_amount: u64,
    pub taker_amount: u64,

    // If swap pool initialized, with data written to it
    pub is_initialized: bool,
    // Bump seed used to generate the program address / authority
    pub bump_seed: u8,
    // Token program ID associated with the swap
    pub token_program_id: Pubkey,
    // Address of token A liquidity account
    pub token_a_account: Pubkey,
    // Address of token B liquidity account
    pub token_b_account: Pubkey,
    // Address of pool token mint
    pub pool_mint: Pubkey,
    // Address of token A mint
    pub token_a_mint: Pubkey,
    // Address of token B mint
    pub token_b_mint: Pubkey,
    // Curve associated with swap
    pub curve: CurveInput,


}


#[error_code]
pub enum SwapError {
    // 0.
    // The account cannot be initialized because it is already being used.
    #[msg("Swap account already in use")]
    AlreadyInUse,
    // The program address provided doesn't match the value generated by the program.
    #[msg("Invalid program address generated from bump seed and key")]
    InvalidProgramAddress,
    // The owner of the input isn't set to the program address generated by the program.
    #[msg("Input account owner is not the program address")]
    InvalidOwner,
    // The owner of the pool token output is set to the program address generated by the program.
    #[msg("Output pool account owner cannot be the program address")]
    InvalidOutputOwner,
    // The deserialization of the account returned something besides State::Mint.
    #[msg("Deserialized account is not an SPL Token mint")]
    ExpectedMint,

    // 5.
    // The deserialization of the account returned something besides State::Account.
    #[msg("Deserialized account is not an SPL Token account")]
    ExpectedAccount,
    // The input token account is empty.
    #[msg("Input token account empty")]
    EmptySupply,
    // The pool token mint has a non-zero supply.
    #[msg("Pool token mint has a non-zero supply")]
    InvalidSupply,
    // The provided token account has a delegate.
    #[msg("Token account has a delegate")]
    InvalidDelegate,
    // The input token is invalid for swap.
    #[msg("InvalidInput")]
    InvalidInput,

    // 10.
    // Address of the provided swap token account is incorrect.
    #[msg("Address of the provided swap token account is incorrect")]
    IncorrectSwapAccount,
    // Address of the provided pool token mint is incorrect
    #[msg("Address of the provided pool token mint is incorrect")]
    IncorrectPoolMint,
    // The output token is invalid for swap.
    #[msg("InvalidOutput")]
    InvalidOutput,
    // General calculation failure due to overflow or underflow
    #[msg("General calculation failure due to overflow or underflow")]
    CalculationFailure,
    // Invalid instruction number passed in.
    #[msg("Invalid instruction")]
    InvalidInstruction,

    // 15.
    // Swap input token accounts have the same mint
    #[msg("Swap input token accounts have the same mint")]
    RepeatedMint,
    // Swap instruction exceeds desired slippage limit
    #[msg("Swap instruction exceeds desired slippage limit")]
    ExceededSlippage,
    // The provided token account has a close authority.
    #[msg("Token account has a close authority")]
    InvalidCloseAuthority,
    // The pool token mint has a freeze authority.
    #[msg("Pool token mint has a freeze authority")]
    InvalidFreezeAuthority,
    // The pool fee token account is incorrect
    #[msg("Pool fee token account incorrect")]
    IncorrectFeeAccount,

    // 20.
    // Given pool token amount results in zero trading tokens
    #[msg("Given pool token amount results in zero trading tokens")]
    ZeroTradingTokens,
    // The fee calculation failed due to overflow, underflow, or unexpected 0
    #[msg("Fee calculation failed due to overflow, underflow, or unexpected 0")]
    FeeCalculationFailure,
    // ConversionFailure
    #[msg("Conversion to u64 failed with an overflow or underflow")]
    ConversionFailure,
    // The provided fee does not match the program owner's constraints
    #[msg("The provided fee does not match the program owner's constraints")]
    InvalidFee,
    // The provided token program does not match the token program expected by the swap
    #[msg("The provided token program does not match the token program expected by the swap")]
    IncorrectTokenProgramId,

    // 25.
    // The provided curve type is not supported by the program owner
    #[msg("The provided curve type is not supported by the program owner")]
    UnsupportedCurveType,
    // The provided curve parameters are invalid
    #[msg("The provided curve parameters are invalid")]
    InvalidCurve,
    // The operation cannot be performed on the given curve
    #[msg("The operation cannot be performed on the given curve")]
    UnsupportedCurveOperation,
}

pub struct SwapConstraints<'a> {
    /// Owner of the program
    pub owner_key: &'a str,
    /// Valid curve types
    pub valid_curve_types: &'a [CurveType],
    
}

pub const SWAP_CONSTRAINTS: Option<SwapConstraints> = {
    #[cfg(feature = "production")]
    {
        Some(SwapConstraints {
            owner_key: OWNER_KEY,
            valid_curve_types: VALID_CURVE_TYPES,
        })
    }
    #[cfg(not(feature = "production"))]
    {
        None
    }
};

impl<'a> SwapConstraints<'a> {
    /// Checks that the provided curve is valid for the given constraints
    pub fn validate_curve(&self, swap_curve: &SwapCurve) -> Result<()> {
        if self
            .valid_curve_types
            .iter()
            .any(|x| *x == swap_curve.curve_type)
        {
            Ok(())
        } else {
            Err(SwapError::UnsupportedCurveType.into())
        }
    }
}

// Context

impl<'info> Initialize<'info> {
    fn into_mint_to_context(&self) ->CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        let cpi_accounts = MintTo {
            mint: self.pool_mint.to_account_info().clone(),
            to: self.destination.to_account_info().clone(),
            authority: self.authority.clone(),
        }

        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }
}

impl<'info> Swap<'info> {
    fn into_transfer_to_swap_source_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.source_info.clone(),
            to: self.swap_source.to_account_info().clone(),
            authority: self.user_transfer_authority.clone()
        };

        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }

    fn into_transfer_to_destination_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.swap_destination.to_account_info().clone(),
            to: self.destination_info.clone(),
            authority: self.authority.clone()
        };

        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }

}

impl<'info> DepositAllTokenTypes<'info> {
    fn into_transfer_to_token_a_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.source_a_info.clone(),
            to: self.token_a.to_account_info().clone(),
            authority: self.user_transfer_authority_info.clone(),
        };
        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }

    fn into_transfer_to_token_b_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.source_b_info.clone(),
            to: self.token_b.to_account_info().clone(),
            authority: self.user_transfer_authority_info.clone(),
        };
        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }

    fn into_mint_to_context(&self) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        let cpi_accounts = MintTo {
            mint: self.pool_mint.to_account_info().clone(),
            to: self.destination.to_account_info().clone(),
            authority: self.authority.clone(),
        };
        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }
}

impl<'info> WithdrawAllTokenTypes<'info> {
    fn into_burn_context(&self) -> CpiContext<'_, '_, '_, 'info, Burn<'info>> {
        let cpi_accounts = Burn{
            mint: self.pool_mint.to_account_info().clone(),
            to: self.source_info.clone(),
            authority: self.user_transfer_authority_info.clone(),
        };
        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }

    fn into_transfer_to_token_a_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.token_a.to_account_info().clone(),
            to: self.dest_token_a_info.clone(),
            authority: self.authority.clone(),
        };
        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }

    fn into_transfer_to_token_b_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.token_b.to_account_info().clone(),
            to: self.dest_token_b_info.clone(),
            authority: self.authority.clone(),
        };
        CpiContext::new(self.token_program.clone(), cpi_accounts)
    }
}


// Utils

#[allow(clippy::too_many_arguments)]
fn check_accounts(
    solswap: &Solswap,
    program_id: &Pubkey,
    solswap_account_info: &AccountInfo,
    authority_info: &AccountInfo,
    token_a_info: &AccountInfo,
    token_b_info: &AccountInfo,
    pool_mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
    user_token_a_info: Option<&AccountInfo>,
    user_token_b_info: Option<&AccountInfo>,
    // pool_fee_account_info: Option<&AccountInfo>,
) -> Result<()> {
    if solswap_account_info.owner != program_id {
        return Err(ProgramError::IncorrectProgramId.into());
    }
    if *authority_info.key != authority_id(program_id, solswap_account_info.key, solswap.bump_seed)? {
        return Err(SwapError::InvalidProgramAddress.into());
    }
    if *token_a_info.key != solswap.token_a_account {
        return Err(SwapError::IncorrectSwapAccount.into());
    }
    if *token_b_info.key != solswap.token_b_account {
        return Err(SwapError::IncorrectSwapAccount.into());
    }
    if *pool_mint_info.key != solswap.pool_mint {
        return Err(SwapError::IncorrectPoolMint.into());
    }
    if *token_program_info.key != solswap.token_program_id {
        return Err(SwapError::IncorrectTokenProgramId.into());
    }
    if let Some(user_token_a_info) = user_token_a_info {
        if token_a_info.key == user_token_a_info.key {
            return Err(SwapError::InvalidInput.into());
        }
    }
    if let Some(user_token_b_info) = user_token_b_info {
        if token_b_info.key == user_token_b_info.key {
            return Err(SwapError::InvalidInput.into());
        }
    }
    
    Ok(())
}

/// Calculates the authority id by generating a program address.
pub fn authority_id(program_id: &Pubkey, my_info: &Pubkey, bump_seed: u8) -> Result<Pubkey> {
    Pubkey::create_program_address(&[&my_info.to_bytes()[..32], &[bump_seed]], program_id)
        .or(Err(SwapError::InvalidProgramAddress.into()))
}

/// Build Curve object and Fee object
pub fn build_curve(curve_input: &CurveInput) -> Result<SwapCurve> {
    let curve_type = CurveType::try_from(curve_input.curve_type).unwrap();
    let culculator: Box<dyn CurveCalculator> = match curve_type {
        CurveType::ConstantProduct => Box::new(ConstantProductCurve {}),
        CurveType::ConstantPrice => Box::new(ConstantPriceCurve {
            token_b_price: curve_input.curve_parameters,
        }),
        CurveType::Stable => Box::new(StableCurve {
            amp: curve_input.curve_parameters,
        }),
        CurveType::Offset => Box::new(OffsetCurve {
            token_b_offset: curve_input.curve_parameters,
        }),
    };
    let curve = SwapCurve {
        curve_type: curve_type,
        calculator: culculator,
    };
    Ok(curve)
}
