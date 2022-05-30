use anyhow::anyhow;
use dashmap::DashMap;
use futures::TryStreamExt;
use futures::{stream, Stream, StreamExt};
use pin_project::pin_project;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::future::Future;
use std::io::BufReader;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc};
use tokio::sync::Mutex;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;
use tonlibjson_rs::Client;
use tower::{Service, ServiceExt};
use uuid::Uuid;

pub struct ClientBuilder {
    config: Value,
    disable_logging: Option<Value>,
}

impl ClientBuilder {
    pub fn from_json_config(config: &Value) -> Self {
        let full_config = json!({
            "@type": "init",
            "options": {
                "@type": "options",
                "config": {
                    "@type": "config",
                    "config": config.to_string(),
                    "use_callbacks_for_network": false,
                    "blockchain_name": "",
                    "ignore_cache": true
                },
                "keystore_type": {
                    "@type": "keyStoreTypeInMemory"
                }
            }
        });

        Self {
            config: full_config,
            disable_logging: None,
        }
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let config: Value = serde_json::from_reader(reader)?;

        return Ok(ClientBuilder::from_json_config(&config));
    }

    pub fn disable_logging(&mut self) -> &mut Self {
        self.disable_logging = Some(json!({
            "@type": "setLogVerbosityLevel",
            "new_verbosity_level": 1
        }));

        self
    }

    pub async fn build(&self) -> anyhow::Result<AsyncClient> {
        #[derive(Deserialize)]
        struct Void {}

        let client = AsyncClient::new();
        if let Some(ref disable_logging) = self.disable_logging {
            client.execute(disable_logging.clone()).await?;
        }

        client.execute(self.config.clone()).await?;

        Ok(client)
    }
}

const MAIN_WORKCHAIN: i64 = -1;
const MAIN_SHARD: i64 = -9223372036854775808;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "@type", rename = "blocks.shortTxId")]
pub struct ShortTxId {
    pub account: String,
    pub hash: String,
    pub lt: String,
    pub mode: u8,
}

#[derive(Debug, Deserialize)]
pub struct TonError {
    code: i32,
    message: String,
}

impl Display for TonError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Ton api error occurred with code {}, message {}",
            self.code, self.message
        )
    }
}

impl Error for TonError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "@type", rename = "ton.blockIdExt")]
pub struct BlockIdExt {
    pub workchain: i64,
    pub shard: String,
    pub seqno: u64,
    pub root_hash: String,
    pub file_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "@type", rename = "blocks.masterchainInfo")]
pub struct MasterchainInfo {
    pub init: BlockIdExt,
    pub last: BlockIdExt,
    pub state_root_hash: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "@type", rename = "internal.transactionId")]
pub struct InternalTransactionId {
    pub hash: String,
    pub lt: String,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(tag = "@type", rename = "accountAddress")]
pub struct AccountAddress {
    account_address: String,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(tag = "@type", rename = "raw.message")]
pub struct RawMessage {
    source: AccountAddress,
    destination: AccountAddress,
    value: String,
    fwd_fee: String,
    ihr_fee: String,
    created_lt: String,
    body_hash: String,
    msg_data: Value, // @todo maybe only msg.dataRaw
                     // @todo deserialize boc
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(tag = "@type", rename = "raw.transaction")]
pub struct RawTransaction {
    pub utime: i64,
    pub data: String,
    pub transaction_id: InternalTransactionId,
    pub fee: String,
    pub storage_fee: String,
    pub other_fee: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_msg: Option<RawMessage>,
    pub out_msgs: Vec<RawMessage>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RawTransactions {
    transactions: Vec<RawTransaction>,
    previous_transaction_id: InternalTransactionId,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "@type", rename = "blocks.getMasterchainInfo")]
pub struct GetMasterchainInfo {}

#[derive(Debug, Serialize, Deserialize)]
pub struct ShardsResponse {
    pub shards: Vec<BlockIdExt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionsResponse {
    pub id: BlockIdExt,
    pub incomplete: bool,
    pub req_count: u32,
    pub transactions: Vec<ShortTxId>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "@type", rename = "blocks.accountTransactionId")]
pub struct AccountTransactionId {
    pub account: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lt: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "@type", rename = "raw.sendMessage")]
pub struct RawSendMessage {
    pub body: String,
}

impl From<&ShortTxId> for AccountTransactionId {
    fn from(v: &ShortTxId) -> Self {
        AccountTransactionId {
            account: v.account.clone(),
            lt: v.lt.clone(),
        }
    }
}

#[pin_project]
#[derive(Clone)]
pub struct AsyncClient {
    client: Arc<Client>,
    responses: Arc<DashMap<String, tokio::sync::oneshot::Sender<Value>>>,
}

impl AsyncClient {
    pub fn new() -> Self {
        let client = Arc::new(Client::new());
        let client_recv = client.clone();

        let responses: Arc<DashMap<String, tokio::sync::oneshot::Sender<Value>>> =
            Arc::new(DashMap::new());
        let responses_rcv = Arc::clone(&responses);

        let _ = thread::spawn(move || {
            let timeout = Duration::from_secs(20);
            loop {
                if let Ok(packet) = client_recv.receive(timeout) {
                    if let Ok(json) = serde_json::from_str::<Value>(packet) {
                        if let Some(Value::String(ref id)) = json.get("@extra") {
                            if let Some((_, s)) = responses_rcv.remove(id) {
                                let _ = s.send(json);
                            }
                        } else {
                            println!("Unexpected response {:?}", json.to_string());
                        }
                    }
                }
            }
        });

        return AsyncClient { client, responses };
    }

    async fn send(&self, request: Value) -> () {
        let _ = self.client.send(&request.to_string());
    }

    pub async fn execute(&self, request: Value) -> anyhow::Result<Value> {
        return self
            .execute_typed_with_timeout(&request, Duration::from_secs(20))
            .await;
    }

    async fn execute_typed_with_timeout<T: DeserializeOwned>(
        &self,
        request: &Value,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let mut request = request.clone();

        let id = Uuid::new_v4().to_string();
        request["@extra"] = json!(id);
        let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
        self.responses.insert(id.clone(), tx);

        let x = request.to_string();
        // println!("{:#?}", x);
        let _ = self.client.send(&x);

        let timeout = tokio::time::timeout(timeout, rx).await?;

        return match timeout {
            Ok(mut value) => {
                // println!("{:#?}", value);
                let obj = value.as_object_mut().ok_or(anyhow!("Not an object"))?;
                let _ = obj.remove("@extra");

                if value["@type"] == "error" {
                    return match serde_json::from_value::<TonError>(value) {
                        Ok(e) => Err(anyhow::Error::from(e)),
                        Err(e) => Err(anyhow::Error::from(e)),
                    };
                }

                serde_json::from_value::<T>(value).map_err(anyhow::Error::from)
            }
            Err(e) => {
                self.responses.remove(&id);

                Err(anyhow::Error::from(e))
            }
        };
    }

    pub async fn synchronize(&self) -> anyhow::Result<Value> {
        let query = json!({
            "@type": "sync"
        });

        return self
            .execute_typed_with_timeout::<Value>(&query, Duration::from_secs(120))
            .await;
    }
}

impl Service<Value> for AsyncClient {
    type Response = Value;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Value) -> Self::Future {
        let this = self.clone();

        return Box::pin(async move { this.execute(req).await });
    }
}

#[derive(Clone)]
pub struct Ton<S> where S: Clone + Send + 'static {
    service: S,
}

impl<S> Ton<S>
where
    S: Service<
        Value,
        Response = Value,
        Error = Box<(dyn std::error::Error + Sync + Send + 'static)>,
    > + Clone + Send,
{
    pub fn new(service: S) -> Self {
        Self { service }
    }

    pub async fn get_masterchain_info(&mut self) -> anyhow::Result<MasterchainInfo> {
        let query = json!(GetMasterchainInfo {});

        let response = self.call(query).await?;

        return Ok(serde_json::from_value(response)?);
    }

    pub async fn look_up_block_by_seqno(
        &mut self,
        workchain: i64,
        shard: i64,
        seqno: u64,
    ) -> anyhow::Result<Value> {
        return self.look_up_block(workchain, shard, seqno, 0).await;
    }

    pub async fn look_up_block_by_lt(
        &mut self,
        workchain: i64,
        shard: i64,
        lt: i64,
    ) -> anyhow::Result<Value> {
        return self.look_up_block(workchain, shard, 0, lt).await;
    }

    pub async fn get_shards(&mut self, master_seqno: u64) -> anyhow::Result<ShardsResponse> {
        let block = self
            .look_up_block(MAIN_WORKCHAIN, MAIN_SHARD, master_seqno, 0)
            .await?;

        let request = json!({
            "@type": "blocks.getShards",
            "id": block
        });

        let response = self.call(request).await?;

        Ok(serde_json::from_value(response)?)
    }

    pub async fn get_block_header(
        &mut self,
        workchain: i64,
        shard: i64,
        seqno: u64,
    ) -> anyhow::Result<Value> {
        let block = self.look_up_block(workchain, shard, seqno, 0).await?;

        let request = json!({
            "@type": "blocks.getBlockHeader",
            "id": block
        });

        return self.call(request).await;
    }

    pub async fn raw_get_account_state(&mut self, address: &str) -> anyhow::Result<Value> {
        let request = json!({
            "@type": "raw.getAccountState",
            "account_address": {
                "account_address": address
            }
        });

        let mut response = self.call(request).await?;

        let code = response["code"].as_str().unwrap_or("");
        let state: &str = if code.len() == 0 || code.parse::<i64>().is_ok() {
            if response["frozen_hash"].as_str().unwrap_or("").len() == 0 {
                "uninitialized"
            } else {
                "frozen"
            }
        } else {
            "active"
        };

        response["state"] = Value::from(state);
        if let Some(balance) = response["balance"].as_i64() {
            if balance < 0 {
                response["balance"] = Value::from(0);
            }
        }

        Ok(response)
    }

    pub async fn get_account_state(&mut self, address: &str) -> anyhow::Result<Value> {
        let request = json!({
            "@type": "getAccountState",
            "account_address": {
                "account_address": address
            }
        });

        return self.call(request).await;
    }

    pub async fn raw_get_transactions(
        &mut self,
        address: &str,
        from_lt: &str,
        from_hash: &str,
    ) -> anyhow::Result<RawTransactions> {
        let request = json!({
            "@type": "raw.getTransactions",
            "account_address": {
                "account_address": address
            },
            "from_transaction_id": {
                "@type": "internal.transactionId",
                "lt": from_lt,
                "hash": from_hash
            }
        });

        let response = self.call(request).await?;

        Ok(serde_json::from_value(response)?)
    }

    async fn blocks_get_transactions(
        &mut self,
        block: &BlockIdExt,
        count: u32,
    ) -> anyhow::Result<TransactionsResponse> {
        self.blocks_get_transactions_after(
            block,
            count,
            AccountTransactionId {
                account: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                lt: "".to_string(),
            },
        )
        .await
    }

    async fn blocks_get_transactions_after(
        &mut self,
        block: &BlockIdExt,
        count: u32,
        tx: AccountTransactionId,
    ) -> anyhow::Result<TransactionsResponse> {
        let request = json!({
            "@type": "blocks.getTransactions",
            "id": block,
            "mode": 7 + 128,
            "count": count,
            "after": tx,
        });

        let response = self.call(request).await?;

        return Ok(serde_json::from_value(response)?);
    }

    async fn look_up_block(
        &mut self,
        workchain: i64,
        shard: i64,
        seqno: u64,
        lt: i64,
    ) -> anyhow::Result<Value> {
        let mut mode: i32 = 0;
        if seqno > 0 {
            mode += 1
        }
        if lt > 0 {
            mode += 2
        }

        let request = json!({
            "@type": "blocks.lookupBlock",
            "mode": mode,
            "id": {
                "@type": "ton.blockId",
                "workchain": workchain,
                "shard": shard,
                "seqno": seqno
            },
            "lt": lt
        });

        return self.call(request).await;
    }

    pub async fn send_message(&mut self, message: &str) -> anyhow::Result<Value> {
        let request = json!(RawSendMessage {
            body: message.to_string()
        });

        return self.call(request).await;
    }

    pub async fn get_tx_stream(
        &mut self,
        block: BlockIdExt,
    ) -> impl Stream<Item = anyhow::Result<ShortTxId>> + '_ where <S as Service<Value>>::Future: Send {
        struct State<'a, Y> where Y: Service<
            Value,
            Response = Value,
            Error = Box<(dyn std::error::Error + Sync + Send + 'static)>,
        > + Clone + Send + 'static {
            last_tx: Option<AccountTransactionId>,
            incomplete: bool,
            block: BlockIdExt,
            this: &'a mut Ton<Y>
        }

        let this = self;
        return stream::try_unfold(
            State {
                last_tx: None,
                incomplete: true,
                block,
                this
            },
            move |state| {
                async move {
                    if state.incomplete == false {
                        return anyhow::Ok(None);
                    }

                    let txs;
                    if let Some(tx) = state.last_tx {
                        txs = state.this.blocks_get_transactions_after(&state.block, 30, tx).await?
                    } else {
                        txs = state.this.blocks_get_transactions(&state.block, 30).await?
                    }

                    println!("got {} transactions", txs.transactions.len());

                    let last_tx = txs.transactions.last().map(AccountTransactionId::from);

                    return anyhow::Ok(Some((
                        stream::iter(txs.transactions.into_iter().map(anyhow::Ok)),
                        State {
                            last_tx,
                            incomplete: txs.incomplete,
                            block: state.block,
                            this: state.this
                        },
                    )));
                }
            },
        )
        .try_flatten();
    }

    pub async fn get_account_tx_stream(
        &mut self,
        address: String,
    ) -> anyhow::Result<impl Stream<Item = anyhow::Result<RawTransaction>> + '_> {
        let account_state = self.raw_get_account_state(&address).await?;
        let ltx = account_state
            .get("last_transaction_id")
            .ok_or(anyhow!("Unexpected missed last_transaction_id"))?;
        let last_tx = serde_json::from_value::<InternalTransactionId>(ltx.to_owned())?;

        return Ok(self.get_account_tx_stream_from(address, last_tx));
    }

    pub fn get_account_tx_stream_from(
        &mut self,
        address: String,
        last_tx: InternalTransactionId,
    ) -> impl Stream<Item = anyhow::Result<RawTransaction>> + '_ {
        struct State<'a, Y> where Y: Service<
            Value,
            Response = Value,
            Error = Box<(dyn std::error::Error + Sync + Send + 'static)>,
        > + Clone + Send + 'static {
            address: String,
            last_tx: InternalTransactionId,
            this: &'a mut Ton<Y>
        }

        let this = self;
        return stream::try_unfold(State { address, last_tx, this }, move |state| async move {
            let txs = state.this
                .raw_get_transactions(&state.address, &state.last_tx.lt, &state.last_tx.hash)
                .await?;

            if let Some(last_tx) = txs.transactions.last() {
                let tx_id = last_tx.transaction_id.clone();
                anyhow::Ok(Some((
                    stream::iter(txs.transactions.into_iter().map(anyhow::Ok)),
                    State {
                        address: state.address,
                        last_tx: tx_id,
                        this: state.this
                    },
                )))
            } else {
                anyhow::Ok(None)
            }
        })
        .try_flatten();
    }

    async fn call(&mut self, request: Value) -> anyhow::Result<Value> {
        let ready = self.service.ready().await.map_err(|e| anyhow!(e))?;
        let call = ready.call(request).await.map_err(|e| anyhow!(e))?;

        return Ok(call);
    }
}
