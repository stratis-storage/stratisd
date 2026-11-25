// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod methods;

use std::sync::Arc;

use zbus::{interface, Connection, Result};

use methods::get_report_method;

use crate::{dbus::consts, engine::Engine};

pub struct ReportInterface {
    engine: Arc<dyn Engine>,
}

impl ReportInterface {
    pub fn new(engine: Arc<dyn Engine>) -> Self {
        ReportInterface { engine }
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

#[interface(name = "org.storage.stratis3.Report", introspection_docs = false)]
impl ReportInterface {
    #[zbus(out_args("result", "return_code", "return_string"))]
    fn get_report(&self, name: &str) -> (String, u16, String) {
        get_report_method(&self.engine, name)
    }
}
