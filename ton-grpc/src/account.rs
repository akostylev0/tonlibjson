use tonic::{async_trait, Request, Response, Status};
use tonlibjson_client::ton::TonClient;
use anyhow::Result;
use crate::helpers::{extend_block_id};
use crate::ton::account_server::Account;
use crate::ton::{GetAccountStateRequest, GetAccountStateResponse, GetShardAccountCellRequest, GetShardAccountCellResponse};
use crate::ton::get_account_state_response::AccountState;
use crate::ton::{get_account_state_request, get_shard_account_cell_request};

pub struct AccountService {
    client: TonClient
}

impl AccountService {
    pub async fn from_env() -> Result<Self> {
        Ok(Self {
            client: TonClient::from_env().await?
        })
    }
}

#[async_trait]
impl Account for AccountService {
    #[tracing::instrument(skip_all, err)]
    async fn get_account_state(&self, request: Request<GetAccountStateRequest>) -> std::result::Result<Response<GetAccountStateResponse>, Status> {
        let msg = request.into_inner();

        let address = msg.account_address
            .ok_or_else(|| Status::invalid_argument("Empty AccountAddress"))?;

        let criteria = match msg.criteria {
            None => {
                let block_id = self.client.get_masterchain_info()
                    .await
                    .map(|i| i.last);

                either::Left(block_id)
            },
            Some(get_account_state_request::Criteria::BlockId(block_id)) => {
                either::Left(extend_block_id(&self.client, &block_id).await)
            },
            Some(get_account_state_request::Criteria::TransactionId(tx_id)) => either::Right(Ok(tx_id.into()))
        }.factor_err().map_err(|e| Status::internal(e.to_string()))?;

        let state = criteria.map_left(|block_id| async {
            self.client.raw_get_account_state_on_block(&address.address, block_id)
                .await
        }).map_right(|tx_id| async {
            self.client.raw_get_account_state_by_transaction(&address.address, tx_id)
                .await
        }).await.map_err(|e| Status::internal(e.to_string()))?;

        let block_id = state.block_id.clone();
        let balance = state.balance.unwrap_or_default();
        let last_transaction_id = state.last_transaction_id.clone().map(|t| t.into());
        let state: AccountState = state.into();
        let block_id = block_id.into();

        Ok(Response::new(GetAccountStateResponse {
            balance,
            account_address: Some(address),
            block_id: Some(block_id),
            last_transaction_id,
            account_state: Some(state)
        }))
    }

    #[tracing::instrument(skip_all, err)]
    async fn get_shard_account_cell(&self, request: Request<GetShardAccountCellRequest>) -> Result<Response<GetShardAccountCellResponse>, Status> {
        let msg = request.into_inner();

        let address = msg.account_address
            .ok_or_else(|| Status::invalid_argument("Empty AccountAddress"))?;

        let criteria = match msg.criteria {
            None => {
                let block_id = self.client.get_masterchain_info()
                    .await
                    .map(|i| i.last);

                either::Left(block_id)
            },
            Some(get_shard_account_cell_request::Criteria::BlockId(block_id)) => {
                either::Left(extend_block_id(&self.client, &block_id).await)
            },
            Some(get_shard_account_cell_request::Criteria::TransactionId(tx_id)) => either::Right(Ok(tx_id.into()))
        }.factor_err().map_err(|e| Status::internal(e.to_string()))?;

        let (block_id, cell) = criteria.map_left(|block_id| async {
            let cell = self.client.get_shard_account_cell_on_block(&address.address, block_id.clone()).await?;

            Ok((block_id, cell))
        }).map_right(|tx_id| async {
            let state = self.client.raw_get_account_state_by_transaction(&address.address, tx_id).await?;
            let cell = self.client.get_shard_account_cell_on_block(&address.address, state.block_id.clone()).await?;

            Ok((state.block_id, cell))
        }).await.map_err(|e: anyhow::Error| Status::internal(e.to_string()))?;

        let block_id = block_id.into();
        let cell = cell.into();

        let response = GetShardAccountCellResponse {
            account_address: Some(address),
            block_id: Some(block_id),
            cell: Some(cell)
        };

        Ok(Response::new(response))
    }
}
