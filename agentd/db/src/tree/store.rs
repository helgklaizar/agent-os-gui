//! `TreeStore` for `TreeRepo`: reads on read connections, writes in one writer transaction.

use agentd_core::{
    AccountInput, FolderInput, PersonaInput, PortFuture, SessionFilter, SessionInput, TreeError,
    TreeResult, TreeStore,
};
use serde_json::{Value, json};

use super::{TreeRepo, accounts, folders, json, personas, sessions};

fn found(v: Option<Value>) -> TreeResult<Value> {
    v.ok_or(TreeError::NotFound)
}

impl TreeStore for TreeRepo {
    fn list_folders(&self) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { Ok(Value::Array(self.read(json::folders).await?)) })
    }

    fn create_folder(&self, input: FolderInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| folders::create(tx, now, input)).await
        })
    }

    fn patch_folder(&self, id: i64, input: FolderInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { self.run(move |tx| folders::patch(tx, id, input)).await })
    }

    fn delete_folder(&self, id: i64) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move { self.run(move |tx| folders::delete(tx, id)).await })
    }

    fn list_sessions(&self, f: SessionFilter) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let rows = self
                .read(move |c| json::sessions(c, f.folder_id, f.parent_id, f.archived))
                .await?;
            Ok(Value::Array(rows))
        })
    }

    fn get_session(&self, id: i64) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { found(self.read(move |c| json::session(c, id)).await?) })
    }

    fn create_session(&self, input: SessionInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let (now, home) = (self.now(), self.home().map(ToOwned::to_owned));
            self.run(move |tx| sessions::create(tx, now, input, home.as_deref()))
                .await
        })
    }

    fn create_thread(&self, parent_id: i64, name: String) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| sessions::create_thread(tx, now, parent_id, name))
                .await
        })
    }

    fn patch_session(&self, id: i64, input: SessionInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let (now, home) = (self.now(), self.home().map(ToOwned::to_owned));
            self.run(move |tx| sessions::patch(tx, now, id, input, home.as_deref()))
                .await
        })
    }

    fn delete_session(&self, id: i64) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move { self.run(move |tx| sessions::delete(tx, id)).await })
    }

    fn mark_read(&self, id: i64) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move { self.run(move |tx| sessions::mark_read(tx, id)).await })
    }

    fn bump_unread(&self, id: i64) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move { self.run(move |tx| sessions::bump_unread(tx, id)).await })
    }

    fn list_personas(&self) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { Ok(Value::Array(self.read(json::personas).await?)) })
    }

    fn create_persona(&self, input: PersonaInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| personas::create(tx, now, input)).await
        })
    }

    fn patch_persona(&self, id: i64, input: PersonaInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| personas::patch(tx, now, id, input))
                .await
        })
    }

    fn delete_persona(&self, id: i64) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| personas::delete(tx, now, id)).await
        })
    }

    fn list_accounts(&self) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { Ok(Value::Array(self.read(json::accounts).await?)) })
    }

    fn create_account(&self, input: AccountInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| accounts::create(tx, now, input)).await
        })
    }

    fn patch_account(&self, id: i64, input: AccountInput) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { self.run(move |tx| accounts::patch(tx, id, input)).await })
    }

    fn delete_account(&self, id: i64) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move {
            let now = self.now();
            self.run(move |tx| accounts::delete(tx, now, id)).await
        })
    }

    fn set_account_status(&self, id: i64, status: &'static str) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move {
            self.run(move |tx| accounts::set_status(tx, id, status))
                .await
        })
    }

    fn link_account_secret(
        &self,
        id: i64,
        secret_ref: Option<String>,
    ) -> PortFuture<'_, TreeResult<()>> {
        Box::pin(async move {
            self.run(move |tx| accounts::link_secret(tx, id, secret_ref))
                .await
        })
    }

    fn sessions_snapshot(&self) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move {
            let (folders, sessions) = self
                .read(|c| Ok((json::folders(c)?, json::sessions(c, None, None, None)?)))
                .await?;
            Ok(json!({"folders": folders, "sessions": sessions}))
        })
    }

    fn connections_snapshot(&self) -> PortFuture<'_, TreeResult<Value>> {
        Box::pin(async move { Ok(json!({"items": self.read(json::accounts).await?})) })
    }
}
