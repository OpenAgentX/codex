use bytes::Bytes;
use serde::Serialize;

use crate::server::config::SocialLoginProvider;
use crate::server::state::DeviceCodeView;
use crate::server::state::TaskPageData;

const INDEX_HTML: &str = include_str!("../../frontend/dist/index.html");
const APP_CSS: &str = include_str!("../../frontend/dist/assets/app.css");
const APP_JS: &str = include_str!("../../frontend/dist/assets/app.js");

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "page", rename_all = "camelCase")]
pub enum PageBootstrap {
    #[serde(rename_all = "camelCase")]
    BrowserLogin {
        continue_to: String,
        username_hint: String,
        password_hint: String,
        error_message: Option<String>,
        social_providers: Vec<SocialLoginProvider>,
    },
    #[serde(rename_all = "camelCase")]
    AccountConfirm {
        continue_to: String,
        email: String,
        account_id: String,
        plan_type: String,
        organization_id: String,
        project_id: String,
        redirect_uri: String,
        oauth_state: String,
    },
    #[serde(rename_all = "camelCase")]
    DeviceAuth {
        records: Vec<DeviceCodeView>,
        message: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    TaskView {
        task: Option<TaskPageData>,
        missing_task_id: Option<String>,
    },
    Callback,
}

pub enum StaticAsset {
    AppCss,
    AppJs,
}

pub fn render_page(bootstrap: &PageBootstrap) -> String {
    INDEX_HTML.replace("__MOCK_BOOTSTRAP__", &bootstrap_json(bootstrap))
}

pub fn asset_bytes(asset: StaticAsset) -> (Bytes, &'static str) {
    match asset {
        StaticAsset::AppCss => (
            Bytes::from_static(APP_CSS.as_bytes()),
            "text/css; charset=utf-8",
        ),
        StaticAsset::AppJs => (
            Bytes::from_static(APP_JS.as_bytes()),
            "application/javascript; charset=utf-8",
        ),
    }
}

fn bootstrap_json(bootstrap: &PageBootstrap) -> String {
    match serde_json::to_string(bootstrap) {
        Ok(json) => json.replace("</script", "<\\/script"),
        Err(_) => "{\"page\":\"callback\"}".to_string(),
    }
}
