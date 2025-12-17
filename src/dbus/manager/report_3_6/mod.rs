// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::{interface, Connection, Result};

use crate::{
    dbus::{consts, manager::report_3_0::get_report_method},
    engine::Engine,
};

pub struct ReportR6 {
    engine: Arc<dyn Engine>,
}

impl ReportR6 {
    pub fn new(engine: Arc<dyn Engine>) -> Self {
        ReportR6 { engine }
    }

    pub async fn register(engine: &Arc<dyn Engine>, connection: &Arc<Connection>) -> Result<()> {
        let report = Self::new(Arc::clone(engine));
        connection
            .object_server()
            .at(consts::STRATIS_BASE_PATH, report)
            .await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.Report.r6", introspection_docs = false)]
impl ReportR6 {
    async fn get_report(&self, name: &str) -> (String, u16, String) {
        get_report_method(&self.engine, name).await
    }
}
