use std::fmt::Debug;
use thiserror::Error;

use sqlparser::ast::{ObjectType, Statement};

use super::fetch::{fetch, fetch_columns};
use super::filter::Filter;
use super::select::select;
use super::update::Update;
use crate::data::{get_table_name, Row, Schema};
use crate::result::Result;
use crate::storage::Store;

#[derive(Error, Debug, PartialEq)]
pub enum ExecuteError {
    #[error("query not supported")]
    QueryNotSupported,

    #[error("drop type not supported")]
    DropTypeNotSupported,
}

#[derive(Debug, PartialEq)]
pub enum Payload {
    Create,
    Insert(Row),
    Select(Vec<Row>),
    Delete(usize),
    Update(usize),
    DropTable,
}

pub fn execute<T: 'static + Debug>(
    storage: &dyn Store<T>,
    sql_query: &Statement,
) -> Result<Payload> {
    match sql_query {
        Statement::CreateTable { name, columns, .. } => {
            let schema = Schema {
                table_name: get_table_name(name)?.clone(),
                column_defs: columns.clone(),
            };

            storage.set_schema(&schema)?;

            Ok(Payload::Create)
        }
        Statement::Query(query) => {
            let rows = select(storage, &query, None)?.collect::<Result<_>>()?;

            Ok(Payload::Select(rows))
        }
        Statement::Insert {
            table_name,
            columns,
            source,
        } => {
            let table_name = get_table_name(table_name)?;
            let Schema { column_defs, .. } = storage.get_schema(table_name)?;
            let key = storage.gen_id(&table_name)?;
            let row = Row::new(column_defs, columns, source)?;
            let row = storage.set_data(&key, row)?;

            Ok(Payload::Insert(row))
        }
        Statement::Update {
            table_name,
            selection,
            assignments,
        } => {
            let table_name = get_table_name(table_name)?;
            let columns = fetch_columns(storage, table_name)?;
            let update = Update::new(storage, table_name, assignments, &columns)?;
            let filter = Filter::new(storage, selection.as_ref(), None);

            let num_rows = fetch(storage, table_name, &columns, filter)?
                .map(|item| {
                    let (_, key, row) = item?;

                    Ok((key, update.apply(row)?))
                })
                .try_fold::<_, _, Result<_>>(0, |num, item: Result<(T, Row)>| {
                    let (key, row) = item?;
                    storage.set_data(&key, row)?;

                    Ok(num + 1)
                })?;

            Ok(Payload::Update(num_rows))
        }
        Statement::Delete {
            table_name,
            selection,
        } => {
            let filter = Filter::new(storage, selection.as_ref(), None);
            let table_name = get_table_name(table_name)?;

            let columns = fetch_columns(storage, table_name)?;
            let num_rows = fetch(storage, table_name, &columns, filter)?
                .try_fold::<_, _, Result<_>>(0, |num: usize, item| {
                    let (_, key, _) = item?;
                    storage.del_data(&key)?;

                    Ok(num + 1)
                })?;

            Ok(Payload::Delete(num_rows))
        }
        Statement::Drop {
            object_type, names, ..
        } => {
            if object_type != &ObjectType::Table {
                return Err(ExecuteError::DropTypeNotSupported.into());
            }

            for name in names {
                let table_name = get_table_name(name)?;

                storage.del_schema(&table_name)?;
            }

            Ok(Payload::DropTable)
        }

        _ => Err(ExecuteError::QueryNotSupported.into()),
    }
}