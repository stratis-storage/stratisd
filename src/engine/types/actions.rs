pub trait EngineActions {
    type Return;
    type Inner;

    fn is_changed(&self) -> bool;
    fn changed(self) -> Option<Self::Return>;
    fn changed_ref(&self) -> Option<&Self::Return>;
    fn unchanged(self) -> Option<Self::Return>;
    fn unchanged_ref(&self) -> Option<&Self::Return>;
    fn destructure(self) -> (Option<Self::Return>, Option<Self::Return>);
    fn into_inner(self) -> Self::Inner;
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateAction<T> {
    Identity(T),
    Created(T),
}

impl<T> EngineActions for CreateAction<T> {
    type Return = T;
    type Inner = T;

    fn is_changed(&self) -> bool {
        match *self {
            CreateAction::Identity(_) => false,
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

    fn unchanged(self) -> Option<T> {
        match self {
            CreateAction::Identity(t) => Some(t),
            _ => None,
        }
    }

    fn unchanged_ref(&self) -> Option<&T> {
        match *self {
            CreateAction::Identity(ref t) => Some(t),
            _ => None,
        }
    }

    fn destructure(self) -> (Option<T>, Option<T>) {
        match self {
            CreateAction::Identity(t) => (None, Some(t)),
            CreateAction::Created(t) => (Some(t), None),
        }
    }

    fn into_inner(self) -> T {
        match self {
            CreateAction::Identity(t) => t,
            CreateAction::Created(t) => t,
        }
    }
}

pub struct SetCreateAction<T> {
    changed: Vec<T>,
    unchanged: Vec<T>,
}

impl<T> SetCreateAction<T> {
    pub fn new(changed: Vec<T>, unchanged: Vec<T>) -> Self {
        SetCreateAction { changed, unchanged }
    }
}

impl<T> EngineActions for SetCreateAction<T> {
    type Return = Vec<T>;
    type Inner = Vec<T>;

    fn is_changed(&self) -> bool {
        self.changed.is_empty()
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

    fn unchanged(self) -> Option<Vec<T>> {
        if self.unchanged.is_empty() {
            None
        } else {
            Some(self.unchanged)
        }
    }

    fn unchanged_ref(&self) -> Option<&Vec<T>> {
        if self.unchanged.is_empty() {
            None
        } else {
            Some(&self.unchanged)
        }
    }

    fn destructure(self) -> (Option<Vec<T>>, Option<Vec<T>>) {
        match (self.changed.is_empty(), self.unchanged.is_empty()) {
            (false, false) => (Some(self.changed), Some(self.unchanged)),
            (true, false) => (None, Some(self.unchanged)),
            (false, true) => (Some(self.changed), None),
            (_, _) => (None, None),
        }
    }

    fn into_inner(self) -> Vec<T> {
        let (mut all, mut unchanged) = (self.changed, self.unchanged);
        all.append(&mut unchanged);
        all
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenameAction<T> {
    Identity(T),
    Renamed(T),
    NoSource,
}

impl<T> EngineActions for RenameAction<T> {
    type Return = T;
    type Inner = Option<T>;

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

    fn unchanged(self) -> Option<T> {
        match self {
            RenameAction::Identity(t) => Some(t),
            _ => None,
        }
    }

    fn unchanged_ref(&self) -> Option<&T> {
        match *self {
            RenameAction::Identity(ref t) => Some(t),
            _ => None,
        }
    }

    fn destructure(self) -> (Option<T>, Option<T>) {
        match self {
            RenameAction::NoSource => (None, None),
            RenameAction::Identity(t) => (None, Some(t)),
            RenameAction::Renamed(t) => (Some(t), None),
        }
    }

    fn into_inner(self) -> Option<T> {
        match self {
            RenameAction::NoSource => None,
            RenameAction::Identity(t) => Some(t),
            RenameAction::Renamed(t) => Some(t),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteAction<T> {
    Identity(T),
    Deleted(T),
}

impl<T> EngineActions for DeleteAction<T> {
    type Return = T;
    type Inner = T;

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

    fn unchanged(self) -> Option<T> {
        match self {
            DeleteAction::Identity(t) => Some(t),
            _ => None,
        }
    }

    fn unchanged_ref(&self) -> Option<&T> {
        match *self {
            DeleteAction::Identity(ref t) => Some(t),
            _ => None,
        }
    }

    fn destructure(self) -> (Option<T>, Option<T>) {
        match self {
            DeleteAction::Identity(t) => (None, Some(t)),
            DeleteAction::Deleted(t) => (Some(t), None),
        }
    }

    fn into_inner(self) -> T {
        match self {
            DeleteAction::Identity(t) => t,
            DeleteAction::Deleted(t) => t,
        }
    }
}

pub type SetDeleteAction<T> = SetCreateAction<T>;
