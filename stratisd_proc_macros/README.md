# `stratisd_proc_macros`

This crate is currently used to hold procedural macros for stratisd used to reduce
boilerplate code through code generation.

## `#[strat_pool_impl_gen]`

This macro is meant to be attached to an `impl... StratPool` item. It will add 

### Attributes

`#[pool_rollback]`: This attribute attached to a method in the `impl` block indicates
that Stratis should monitor the return value of this method for potential rollback
failures and, if detected, should put the pool in maintenance only mode until the
rollback failure is corrected.

`#[pool_mutatating_action]`: This attribute attached to a method in the `impl` block
indicates that this action can mutate the internal state of the pool. This includes
any changes to the underlying data structures or metadata on the device. All methods
that take a `&mut self` reference are by definition mutating actions. However,
there may also be methods like `rebind_clevis` that are also mutating actions
as they cause the LUKS2 metadata for encrypted devices to change.
