// Copyright 2023 Lance Developers.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use async_trait::async_trait;
use lance::io::ObjectStoreParams;
use neon::prelude::*;
use object_store::aws::{AwsCredential, AwsCredentialProvider};
use object_store::CredentialProvider;
use once_cell::sync::OnceCell;
use tokio::runtime::Runtime;

use lancedb::connect;
use lancedb::connection::Connection;
use lancedb::table::ReadParams;

use crate::error::ResultExt;
use crate::query::JsQuery;
use crate::table::JsTable;

mod arrow;
mod convert;
mod error;
mod index;
mod neon_ext;
mod query;
mod table;

struct JsDatabase {
    database: Connection,
}

impl Finalize for JsDatabase {}

// TODO: object_store didn't export this type so I copied it.
// Make a request to object_store to export this type
#[derive(Debug)]
pub struct StaticCredentialProvider<T> {
    credential: Arc<T>,
}

impl<T> StaticCredentialProvider<T> {
    pub fn new(credential: T) -> Self {
        Self {
            credential: Arc::new(credential),
        }
    }
}

#[async_trait]
impl<T> CredentialProvider for StaticCredentialProvider<T>
where
    T: std::fmt::Debug + Send + Sync,
{
    type Credential = T;

    async fn get_credential(&self) -> object_store::Result<Arc<T>> {
        Ok(Arc::clone(&self.credential))
    }
}

fn runtime<'a, C: Context<'a>>(cx: &mut C) -> NeonResult<&'static Runtime> {
    static RUNTIME: OnceCell<Runtime> = OnceCell::new();
    static LOG: OnceCell<()> = OnceCell::new();

    LOG.get_or_init(env_logger::init);

    RUNTIME.get_or_try_init(|| Runtime::new().or_throw(cx))
}

fn database_new(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let path = cx.argument::<JsString>(0)?.value(&mut cx);
    let aws_creds = get_aws_creds(&mut cx, 1)?;
    let region = get_aws_region(&mut cx, 4)?;
    let read_consistency_interval = cx
        .argument_opt(5)
        .and_then(|arg| arg.downcast::<JsNumber, _>(&mut cx).ok())
        .map(|v| v.value(&mut cx))
        .map(std::time::Duration::from_secs_f64);

    let rt = runtime(&mut cx)?;
    let channel = cx.channel();
    let (deferred, promise) = cx.promise();

    let mut conn_builder = connect(&path);
    if let Some(region) = region {
        conn_builder = conn_builder.region(&region);
    }
    if let Some(aws_creds) = aws_creds {
        conn_builder = conn_builder.aws_creds(AwsCredential {
            key_id: aws_creds.key_id,
            secret_key: aws_creds.secret_key,
            token: aws_creds.token,
        });
    }
    if let Some(interval) = read_consistency_interval {
        conn_builder = conn_builder.read_consistency_interval(interval);
    }
    rt.spawn(async move {
        let database = conn_builder.execute().await;

        deferred.settle_with(&channel, move |mut cx| {
            let db = JsDatabase {
                database: database.or_throw(&mut cx)?,
            };
            Ok(cx.boxed(db))
        });
    });
    Ok(promise)
}

fn database_table_names(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let db = cx
        .this()
        .downcast_or_throw::<JsBox<JsDatabase>, _>(&mut cx)?;

    let rt = runtime(&mut cx)?;
    let (deferred, promise) = cx.promise();
    let channel = cx.channel();
    let database = db.database.clone();

    rt.spawn(async move {
        let tables_rst = database.table_names().execute().await;

        deferred.settle_with(&channel, move |mut cx| {
            let tables = tables_rst.or_throw(&mut cx)?;
            let table_names = convert::vec_str_to_array(&tables, &mut cx);
            table_names
        });
    });
    Ok(promise)
}

/// Get AWS creds arguments from the context
/// Consumes 3 arguments
fn get_aws_creds(
    cx: &mut FunctionContext,
    arg_starting_location: i32,
) -> NeonResult<Option<AwsCredential>> {
    let secret_key_id = cx
        .argument_opt(arg_starting_location)
        .filter(|arg| arg.is_a::<JsString, _>(cx))
        .and_then(|arg| arg.downcast_or_throw::<JsString, FunctionContext>(cx).ok())
        .map(|v| v.value(cx));

    let secret_key = cx
        .argument_opt(arg_starting_location + 1)
        .filter(|arg| arg.is_a::<JsString, _>(cx))
        .and_then(|arg| arg.downcast_or_throw::<JsString, FunctionContext>(cx).ok())
        .map(|v| v.value(cx));

    let temp_token = cx
        .argument_opt(arg_starting_location + 2)
        .filter(|arg| arg.is_a::<JsString, _>(cx))
        .and_then(|arg| arg.downcast_or_throw::<JsString, FunctionContext>(cx).ok())
        .map(|v| v.value(cx));

    match (secret_key_id, secret_key, temp_token) {
        (Some(key_id), Some(key), optional_token) => Ok(Some(AwsCredential {
            key_id,
            secret_key: key,
            token: optional_token,
        })),
        (None, None, None) => Ok(None),
        _ => cx.throw_error("Invalid credentials configuration"),
    }
}

fn get_aws_credential_provider(
    cx: &mut FunctionContext,
    arg_starting_location: i32,
) -> NeonResult<Option<AwsCredentialProvider>> {
    Ok(get_aws_creds(cx, arg_starting_location)?.map(|aws_cred| {
        Arc::new(StaticCredentialProvider::new(aws_cred))
            as Arc<dyn CredentialProvider<Credential = AwsCredential>>
    }))
}

/// Get AWS region arguments from the context
fn get_aws_region(cx: &mut FunctionContext, arg_location: i32) -> NeonResult<Option<String>> {
    let region = cx
        .argument_opt(arg_location)
        .filter(|arg| arg.is_a::<JsString, _>(cx))
        .map(|arg| arg.downcast_or_throw::<JsString, FunctionContext>(cx));

    match region {
        Some(Ok(region)) => Ok(Some(region.value(cx))),
        None => Ok(None),
        Some(Err(e)) => Err(e),
    }
}

fn database_open_table(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let db = cx
        .this()
        .downcast_or_throw::<JsBox<JsDatabase>, _>(&mut cx)?;
    let table_name = cx.argument::<JsString>(0)?.value(&mut cx);

    let aws_creds = get_aws_credential_provider(&mut cx, 1)?;

    let aws_region = get_aws_region(&mut cx, 4)?;

    let params = ReadParams {
        store_options: Some(ObjectStoreParams::with_aws_credentials(
            aws_creds, aws_region,
        )),
        ..ReadParams::default()
    };

    let rt = runtime(&mut cx)?;
    let channel = cx.channel();
    let database = db.database.clone();

    let (deferred, promise) = cx.promise();
    rt.spawn(async move {
        let table_rst = database
            .open_table(&table_name)
            .lance_read_params(params)
            .execute()
            .await;

        deferred.settle_with(&channel, move |mut cx| {
            let js_table = JsTable::from(table_rst.or_throw(&mut cx)?);
            Ok(cx.boxed(js_table))
        });
    });
    Ok(promise)
}

fn database_drop_table(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let db = cx
        .this()
        .downcast_or_throw::<JsBox<JsDatabase>, _>(&mut cx)?;
    let table_name = cx.argument::<JsString>(0)?.value(&mut cx);

    let rt = runtime(&mut cx)?;
    let channel = cx.channel();
    let database = db.database.clone();

    let (deferred, promise) = cx.promise();
    rt.spawn(async move {
        let result = database.drop_table(&table_name).await;
        deferred.settle_with(&channel, move |mut cx| {
            result.or_throw(&mut cx)?;
            Ok(cx.null())
        });
    });
    Ok(promise)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("databaseNew", database_new)?;
    cx.export_function("databaseTableNames", database_table_names)?;
    cx.export_function("databaseOpenTable", database_open_table)?;
    cx.export_function("databaseDropTable", database_drop_table)?;
    cx.export_function("tableSearch", JsQuery::js_search)?;
    cx.export_function("tableCreate", JsTable::js_create)?;
    cx.export_function("tableAdd", JsTable::js_add)?;
    cx.export_function("tableCountRows", JsTable::js_count_rows)?;
    cx.export_function("tableDelete", JsTable::js_delete)?;
    cx.export_function("tableUpdate", JsTable::js_update)?;
    cx.export_function("tableMergeInsert", JsTable::js_merge_insert)?;
    cx.export_function("tableCleanupOldVersions", JsTable::js_cleanup)?;
    cx.export_function("tableCompactFiles", JsTable::js_compact)?;
    cx.export_function("tableListIndices", JsTable::js_list_indices)?;
    cx.export_function("tableIndexStats", JsTable::js_index_stats)?;
    cx.export_function(
        "tableCreateScalarIndex",
        index::scalar::table_create_scalar_index,
    )?;
    cx.export_function(
        "tableCreateVectorIndex",
        index::vector::table_create_vector_index,
    )?;
    cx.export_function("tableSchema", JsTable::js_schema)?;
    cx.export_function("tableAddColumns", JsTable::js_add_columns)?;
    cx.export_function("tableAlterColumns", JsTable::js_alter_columns)?;
    cx.export_function("tableDropColumns", JsTable::js_drop_columns)?;
    Ok(())
}
