/// Replacement interpreter paths. `None` or a blank value clears the pin so
/// the conventional command name is used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateScriptInterpreterSettings {
    pub python: Option<String>,
    pub powershell: Option<String>,
    pub batch: Option<String>,
    pub go: Option<String>,
}

const SCRIPT_INTERPRETER_KEYS: [&str; 4] = [
    crate::SCRIPT_INTERPRETER_PYTHON_KEY,
    crate::SCRIPT_INTERPRETER_POWERSHELL_KEY,
    crate::SCRIPT_INTERPRETER_BATCH_KEY,
    crate::SCRIPT_INTERPRETER_GO_KEY,
];

impl AppUseCase {
    /// Interpreters scripts are launched with. A setting that cannot be read
    /// falls back to the conventional command name rather than blocking the
    /// run.
    pub async fn script_interpreter_config(&self) -> crate::scripts::runner::InterpreterConfig {
        match self.load_script_interpreter_config().await {
            Ok(config) => config,
            Err(error) => {
                warn!(error = %error, "failed to read script interpreter settings");
                crate::scripts::runner::InterpreterConfig::default()
            }
        }
    }

    async fn load_script_interpreter_config(
        &self,
    ) -> AppResult<crate::scripts::runner::InterpreterConfig> {
        let [python, powershell, batch, go] = SCRIPT_INTERPRETER_KEYS;
        Ok(crate::scripts::runner::InterpreterConfig {
            python: self.read_script_interpreter(python).await?,
            powershell: self.read_script_interpreter(powershell).await?,
            batch: self.read_script_interpreter(batch).await?,
            go: self.read_script_interpreter(go).await?,
        })
    }

    async fn read_script_interpreter(&self, key_name: &str) -> AppResult<Option<PathBuf>> {
        Ok(
            normalize_optional_string(self.read_setting_string_value(key_name, None).await?)
                .map(PathBuf::from),
        )
    }

    pub async fn get_script_interpreter_settings(
        &self,
        actor: &User,
    ) -> AppResult<crate::scripts::runner::InterpreterConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_script_interpreter_config().await
    }

    pub async fn update_script_interpreter_settings(
        &self,
        actor: &User,
        input: UpdateScriptInterpreterSettings,
    ) -> AppResult<crate::scripts::runner::InterpreterConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let values = [input.python, input.powershell, input.batch, input.go];
        for (key_name, value) in SCRIPT_INTERPRETER_KEYS.into_iter().zip(values) {
            match normalize_optional_string(value) {
                Some(path) => {
                    self.upsert_system_setting_json(key_name, &path, Some(actor.id.clone()))
                        .await?;
                }
                None => self.delete_system_setting(key_name).await?,
            }
        }

        self.emit_settings_saved(
            actor,
            "script_interpreter_settings",
            None,
            SCRIPT_INTERPRETER_KEYS
                .iter()
                .map(|key| key.to_string())
                .collect(),
        )
        .await;

        self.load_script_interpreter_config().await
    }
}
