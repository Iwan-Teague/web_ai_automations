//! Site-specific setup and adapter construction.
//!
//! The task runner should not know how Arena, ChatGPT, Claude, or any other
//! site opens model pickers, selects models, or verifies page state. It asks
//! for a workflow for the selected site and receives a prepared adapter plus
//! the watchdog/recovery hints the FSM needs.

use std::error::Error;
use std::sync::Arc;

use crate::session::{SessionConfig, WebUiTarget};
use crate::webui::adapter::WebUiAdapter;
use crate::webui::arena::{
    ARENA_TITLE_SUBSTR, ARENA_URL_SUBSTR, ArenaAdapter, ArenaSurface, BROWSER_TITLE_SUBSTR,
    session_setup,
};
use crate::webui::deepseek::{
    BROWSER_TITLE_SUBSTR as DEEPSEEK_BROWSER_TITLE_SUBSTR, DEEPSEEK_TITLE_SUBSTR,
    DEEPSEEK_URL_SUBSTR, DeepSeekAdapter, session_setup as deepseek_session_setup,
};
use crate::webui::null::NullAdapter;
use crate::webui::recovery::SiteHints;

pub struct PreparedSite {
    pub adapter: Arc<dyn WebUiAdapter>,
    pub hints: SiteHints,
}

pub trait SiteWorkflow {
    fn target(&self) -> WebUiTarget;
    fn adapter_name(&self) -> &'static str {
        self.target().adapter_name()
    }
    fn prepare(&self, cfg: &SessionConfig) -> Result<PreparedSite, Box<dyn Error>>;
    fn build_adapter_for_debate(
        &self,
        cfg: &SessionConfig,
    ) -> Result<(Arc<dyn WebUiAdapter>, SiteHints), Box<dyn Error>>;
}

pub struct ArenaWorkflow;

impl ArenaWorkflow {
    fn hints() -> SiteHints {
        SiteHints {
            browser_title_substr: BROWSER_TITLE_SUBSTR,
            site_url: ARENA_URL_SUBSTR,
            site_title_substr: ARENA_TITLE_SUBSTR,
        }
    }

    fn adapter(cfg: &SessionConfig) -> Arc<ArenaAdapter> {
        Arc::new(ArenaAdapter::new_for_surface(
            ArenaSurface::from_prompt_mode(cfg.prompt_mode),
        ))
    }
}

impl SiteWorkflow for ArenaWorkflow {
    fn target(&self) -> WebUiTarget {
        WebUiTarget::Arena
    }

    fn prepare(&self, cfg: &SessionConfig) -> Result<PreparedSite, Box<dyn Error>> {
        let arena = Self::adapter(cfg);
        match session_setup::run(arena.as_ref(), cfg) {
            Ok(rep) => println!(
                "[setup] browser_focused={} fullscreen={} \
                 navigated={} (reused_tab={}) sidebar_collapsed={} \
                 new_chat={} direct_mode={} model_selected={}",
                rep.browser_focused,
                rep.forced_fullscreen,
                rep.navigated_to_arena,
                rep.reused_existing_tab,
                rep.sidebar_collapsed,
                rep.clicked_new_chat,
                rep.forced_direct_mode,
                rep.model_selected,
            ),
            Err(e) => {
                eprintln!("[setup] FATAL: {e}");
                eprintln!("[setup] aborting session — refusing to start FSM with unverified state");
                return Err(format!("session setup failed: {e}").into());
            }
        }

        Ok(PreparedSite {
            adapter: arena as Arc<dyn WebUiAdapter>,
            hints: Self::hints(),
        })
    }

    fn build_adapter_for_debate(
        &self,
        cfg: &SessionConfig,
    ) -> Result<(Arc<dyn WebUiAdapter>, SiteHints), Box<dyn Error>> {
        Ok((Self::adapter(cfg) as Arc<dyn WebUiAdapter>, Self::hints()))
    }
}

pub struct NullWorkflow;

impl NullWorkflow {
    fn hints() -> SiteHints {
        SiteHints {
            browser_title_substr: "",
            site_url: "",
            site_title_substr: "",
        }
    }
}

impl SiteWorkflow for NullWorkflow {
    fn target(&self) -> WebUiTarget {
        WebUiTarget::Null
    }

    fn prepare(&self, _cfg: &SessionConfig) -> Result<PreparedSite, Box<dyn Error>> {
        Ok(PreparedSite {
            adapter: Arc::new(NullAdapter::new()) as Arc<dyn WebUiAdapter>,
            hints: Self::hints(),
        })
    }

    fn build_adapter_for_debate(
        &self,
        _cfg: &SessionConfig,
    ) -> Result<(Arc<dyn WebUiAdapter>, SiteHints), Box<dyn Error>> {
        Ok((
            Arc::new(NullAdapter::new()) as Arc<dyn WebUiAdapter>,
            Self::hints(),
        ))
    }
}

pub struct DeepSeekWorkflow;

impl DeepSeekWorkflow {
    fn hints() -> SiteHints {
        SiteHints {
            browser_title_substr: DEEPSEEK_BROWSER_TITLE_SUBSTR,
            site_url: DEEPSEEK_URL_SUBSTR,
            site_title_substr: DEEPSEEK_TITLE_SUBSTR,
        }
    }

    fn adapter() -> Arc<DeepSeekAdapter> {
        Arc::new(DeepSeekAdapter::new())
    }
}

impl SiteWorkflow for DeepSeekWorkflow {
    fn target(&self) -> WebUiTarget {
        WebUiTarget::DeepSeek
    }

    fn prepare(&self, cfg: &SessionConfig) -> Result<PreparedSite, Box<dyn Error>> {
        let deepseek = Self::adapter();
        match deepseek_session_setup::run(deepseek.as_ref(), cfg) {
            Ok(rep) => println!(
                "[setup] deepseek browser_focused={} navigated={} reused_tab={} fullscreen={} new_chat={} model_selected={} deep_thinking={} smart_search={}",
                rep.browser_focused,
                rep.navigated_to_deepseek,
                rep.reused_existing_tab,
                rep.forced_fullscreen,
                rep.clicked_new_chat,
                rep.model_selected,
                rep.deep_thinking_set,
                rep.smart_search_set,
            ),
            Err(e) => {
                eprintln!("[setup] FATAL: {e}");
                return Err(format!("DeepSeek session setup failed: {e}").into());
            }
        }

        Ok(PreparedSite {
            adapter: deepseek as Arc<dyn WebUiAdapter>,
            hints: Self::hints(),
        })
    }

    fn build_adapter_for_debate(
        &self,
        _cfg: &SessionConfig,
    ) -> Result<(Arc<dyn WebUiAdapter>, SiteHints), Box<dyn Error>> {
        Ok((Self::adapter() as Arc<dyn WebUiAdapter>, Self::hints()))
    }
}

pub fn workflow_for(target: WebUiTarget) -> Result<Box<dyn SiteWorkflow>, Box<dyn Error>> {
    match target {
        WebUiTarget::Arena => Ok(Box::new(ArenaWorkflow)),
        WebUiTarget::DeepSeek => Ok(Box::new(DeepSeekWorkflow)),
        WebUiTarget::Null => Ok(Box::new(NullWorkflow)),
        WebUiTarget::ChatGpt | WebUiTarget::Claude => Err(format!(
            "{} workflow is not implemented yet. Add a src/webui/{}/ adapter and SiteWorkflow implementation first.",
            target,
            target.adapter_name()
        )
        .into()),
    }
}

pub fn prepare_site(cfg: &SessionConfig) -> Result<PreparedSite, Box<dyn Error>> {
    workflow_for(cfg.target_webui)?.prepare(cfg)
}

pub fn build_debate_adapter(
    target: WebUiTarget,
    cfg: &SessionConfig,
) -> Result<(Arc<dyn WebUiAdapter>, SiteHints), Box<dyn Error>> {
    workflow_for(target)?.build_adapter_for_debate(cfg)
}
