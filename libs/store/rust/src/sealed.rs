use std::time::SystemTime;

use crate::entries::StoreError;
use crate::id::Id;

pub trait Sealed {
    fn sealed_at(&self, id: Id) -> Result<Option<SystemTime>, StoreError>;
}
