use types::StratisResult;

pub trait Pool {
    fn add_blockdev(&mut self, path: &str) -> StratisResult<()>;
    fn add_cachedev(&mut self, path: &str) -> StratisResult<()>;
    fn destroy(&mut self) -> StratisResult<()>;
}

pub trait Engine {
    fn create_pool(&self,
                   name: &str,
                   blockdev_paths: &[&str],
                   raid_level: i32)
                   -> StratisResult<Box<Pool>>;

    fn destroy_pool(&self, name: &str) -> StratisResult<()>;
    fn list_pools(&self) -> StratisResult<()>;
}
