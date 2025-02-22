//! Database debugging tool
use crate::{
    args::StageEnum,
    dirs::{DataDirPath, MaybePlatformPath},
    utils::DbTool,
};
use clap::Parser;
use reth_db::{
    database::Database,
    mdbx::{Env, WriteMap},
    tables,
    transaction::DbTxMut,
};
use reth_primitives::ChainSpec;
use reth_staged_sync::utils::{chainspec::genesis_value_parser, init::insert_genesis_state};
use reth_stages::stages::{
    ACCOUNT_HASHING, EXECUTION, INDEX_ACCOUNT_HISTORY, INDEX_STORAGE_HISTORY, MERKLE_EXECUTION,
    MERKLE_UNWIND, STORAGE_HASHING,
};
use std::{path::PathBuf, sync::Arc};
use tracing::info;

/// `reth drop-stage` command
#[derive(Debug, Parser)]
pub struct Command {
    /// The path to the data dir for all reth files and subdirectories.
    ///
    /// Defaults to the OS-specific data directory:
    ///
    /// - Linux: `$XDG_DATA_HOME/reth/` or `$HOME/.local/share/reth/`
    /// - Windows: `{FOLDERID_RoamingAppData}/reth/`
    /// - macOS: `$HOME/Library/Application Support/reth/`
    #[arg(long, value_name = "DATA_DIR", verbatim_doc_comment, default_value_t)]
    datadir: MaybePlatformPath<DataDirPath>,

    /// The path to the database folder. If not specified, it will be set in the data dir for the
    /// chain being used.
    #[arg(long, value_name = "PATH", verbatim_doc_comment)]
    db: Option<PathBuf>,

    /// The chain this node is running.
    ///
    /// Possible values are either a built-in chain or the path to a chain specification file.
    ///
    /// Built-in chains:
    /// - mainnet
    /// - goerli
    /// - sepolia
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        verbatim_doc_comment,
        default_value = "mainnet",
        value_parser = genesis_value_parser
    )]
    chain: Arc<ChainSpec>,

    stage: StageEnum,
}

impl Command {
    /// Execute `db` command
    pub async fn execute(self) -> eyre::Result<()> {
        // add network name to data dir
        let data_dir = self.datadir.unwrap_or_chain_default(self.chain.chain);

        // use the overridden db path if specified
        let db_path = self.db.clone().unwrap_or(data_dir.db_path());

        std::fs::create_dir_all(&db_path)?;

        let db = Env::<WriteMap>::open(db_path.as_ref(), reth_db::mdbx::EnvKind::RW)?;

        let tool = DbTool::new(&db)?;

        match &self.stage {
            StageEnum::Execution => {
                tool.db.update(|tx| {
                    tx.clear::<tables::PlainAccountState>()?;
                    tx.clear::<tables::PlainStorageState>()?;
                    tx.clear::<tables::AccountChangeSet>()?;
                    tx.clear::<tables::StorageChangeSet>()?;
                    tx.clear::<tables::Bytecodes>()?;
                    tx.put::<tables::SyncStage>(EXECUTION.0.to_string(), 0)?;
                    insert_genesis_state::<Env<WriteMap>>(tx, self.chain.genesis())?;
                    Ok::<_, eyre::Error>(())
                })??;
            }
            StageEnum::Hashing => {
                tool.db.update(|tx| {
                    // Clear hashed accounts
                    tx.clear::<tables::HashedAccount>()?;
                    tx.put::<tables::SyncStageProgress>(ACCOUNT_HASHING.0.into(), Vec::new())?;
                    tx.put::<tables::SyncStage>(ACCOUNT_HASHING.0.to_string(), 0)?;

                    // Clear hashed storages
                    tx.clear::<tables::HashedStorage>()?;
                    tx.put::<tables::SyncStageProgress>(STORAGE_HASHING.0.into(), Vec::new())?;
                    tx.put::<tables::SyncStage>(STORAGE_HASHING.0.to_string(), 0)?;

                    Ok::<_, eyre::Error>(())
                })??;
            }
            StageEnum::Merkle => {
                tool.db.update(|tx| {
                    tx.clear::<tables::AccountsTrie>()?;
                    tx.clear::<tables::StoragesTrie>()?;
                    tx.put::<tables::SyncStage>(MERKLE_EXECUTION.0.to_string(), 0)?;
                    tx.put::<tables::SyncStage>(MERKLE_UNWIND.0.to_string(), 0)?;
                    tx.delete::<tables::SyncStageProgress>(MERKLE_EXECUTION.0.into(), None)?;
                    Ok::<_, eyre::Error>(())
                })??;
            }
            StageEnum::History => {
                tool.db.update(|tx| {
                    tx.clear::<tables::AccountHistory>()?;
                    tx.clear::<tables::StorageHistory>()?;
                    tx.put::<tables::SyncStage>(INDEX_ACCOUNT_HISTORY.0.to_string(), 0)?;
                    tx.put::<tables::SyncStage>(INDEX_STORAGE_HISTORY.0.to_string(), 0)?;
                    Ok::<_, eyre::Error>(())
                })??;
            }
            _ => {
                info!("Nothing to do for stage {:?}", self.stage);
            }
        }

        Ok(())
    }
}
