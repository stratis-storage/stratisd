pub trait EngineAction {
    type Return;

    fn is_changed(&self) -> bool;
    fn changed(self) -> Option<Self::Return>;
    fn changed_ref(&self) -> Option<&Self::Return>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateAction<T> {
    Identity,
    Created(T),
}

impl<T> EngineAction for CreateAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        match *self {
            CreateAction::Identity => false,
            _ => true,
        }
    }

    fn changed(self) -> Option<T> {
        match self {
            CreateAction::Created(t) => Some(t),
            _ => None,
        }
    }

    fn changed_ref(&self) -> Option<&T> {
        match *self {
            CreateAction::Created(ref t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SetCreateAction<T> {
    changed: Vec<T>,
}

impl<T> SetCreateAction<T> {
    pub fn new(changed: Vec<T>) -> Self {
        SetCreateAction { changed }
    }
}

impl<T> EngineAction for SetCreateAction<T> {
    type Return = Vec<T>;

    fn is_changed(&self) -> bool {
        !self.changed.is_empty()
    }

    fn changed(self) -> Option<Vec<T>> {
        if self.changed.is_empty() {
            None
        } else {
            Some(self.changed)
        }
    }

    fn changed_ref(&self) -> Option<&Vec<T>> {
        if self.changed.is_empty() {
            None
        } else {
            Some(&self.changed)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenameAction<T> {
    Identity,
    Renamed(T),
    NoSource,
}

impl<T> EngineAction for RenameAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        match *self {
            RenameAction::Renamed(_) => true,
            _ => false,
        }
    }

    fn changed(self) -> Option<T> {
        match self {
            RenameAction::Renamed(t) => Some(t),
            _ => None,
        }
    }

    fn changed_ref(&self) -> Option<&T> {
        match *self {
            RenameAction::Renamed(ref t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteAction<T> {
    Identity,
    Deleted(T),
}

impl<T> EngineAction for DeleteAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        match *self {
            DeleteAction::Deleted(_) => true,
            _ => false,
        }
    }

    fn changed(self) -> Option<T> {
        match self {
            DeleteAction::Deleted(t) => Some(t),
            _ => None,
        }
    }

    fn changed_ref(&self) -> Option<&T> {
        match *self {
            DeleteAction::Deleted(ref t) => Some(t),
            _ => None,
        }
    }
}

pub type SetDeleteAction<T> = SetCreateAction<T>;
