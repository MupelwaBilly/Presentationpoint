use crate::AppError;

const HISTORY_LIMIT: usize = 200;

/// Reversible document mutations pass through this interface and the command bus.
pub trait Command<D> {
    fn apply(&mut self, document: &mut D) -> Result<(), AppError>;
    fn undo(&mut self, document: &mut D) -> Result<(), AppError>;
}

/// Owns document state and its per-document undo/redo history.
pub struct CommandBus<D> {
    document: D,
    undo: Vec<Box<dyn Command<D>>>,
    redo: Vec<Box<dyn Command<D>>>,
}

impl<D> CommandBus<D> {
    pub fn new(document: D) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn document(&self) -> &D {
        &self.document
    }

    pub fn execute<C>(&mut self, mut command: C) -> Result<(), AppError>
    where
        C: Command<D> + 'static,
    {
        command.apply(&mut self.document)?;
        self.undo.push(Box::new(command));
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        Ok(())
    }

    pub fn undo(&mut self) -> Result<bool, AppError> {
        let Some(mut command) = self.undo.pop() else {
            return Ok(false);
        };
        if let Err(error) = command.undo(&mut self.document) {
            self.undo.push(command);
            return Err(error);
        }
        self.redo.push(command);
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, AppError> {
        let Some(mut command) = self.redo.pop() else {
            return Ok(false);
        };
        if let Err(error) = command.apply(&mut self.document) {
            self.redo.push(command);
            return Err(error);
        }
        self.undo.push(command);
        Ok(true)
    }

    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
}

/// Replaces a document value reversibly, including the empty-document state.
pub struct ReplaceState<T> {
    next: Option<T>,
    previous: Option<T>,
}

impl<T> ReplaceState<T> {
    pub fn new(next: T) -> Self {
        Self {
            next: Some(next),
            previous: None,
        }
    }
}

impl<T> Command<T> for ReplaceState<T> {
    fn apply(&mut self, document: &mut T) -> Result<(), AppError> {
        let Some(next) = self.next.take() else {
            return Err(AppError::CommandInvariant("replacement has no next state"));
        };
        self.previous = Some(std::mem::replace(document, next));
        Ok(())
    }

    fn undo(&mut self, document: &mut T) -> Result<(), AppError> {
        let Some(previous) = self.previous.take() else {
            return Err(AppError::CommandInvariant(
                "replacement has no previous state",
            ));
        };
        self.next = Some(std::mem::replace(document, previous));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, CommandBus};
    use crate::AppError;

    struct Add(i32);

    impl Command<i32> for Add {
        fn apply(&mut self, document: &mut i32) -> Result<(), AppError> {
            *document += self.0;
            Ok(())
        }

        fn undo(&mut self, document: &mut i32) -> Result<(), AppError> {
            *document -= self.0;
            Ok(())
        }
    }

    #[test]
    fn command_bus_applies_undoes_and_redoes_mutations() {
        let mut bus = CommandBus::new(0);
        assert!(bus.execute(Add(7)).is_ok());
        assert_eq!(*bus.document(), 7);
        assert!(matches!(bus.undo(), Ok(true)));
        assert_eq!(*bus.document(), 0);
        assert!(matches!(bus.redo(), Ok(true)));
        assert_eq!(*bus.document(), 7);
    }
}
