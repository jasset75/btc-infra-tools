use anyhow::Result;

pub trait ServiceAdapter {
    fn status(&self, _service: &str) -> Result<String>;
    fn start(&self, _service: &str) -> Result<()>;
    fn stop(&self, _service: &str) -> Result<()>;
    fn restart(&self, _service: &str) -> Result<()>;
}
