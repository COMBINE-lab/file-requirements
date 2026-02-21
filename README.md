# file-requirements

**NOTE**: This crate was primarily written by `Codex-5.3`

`file-requirements` provides a small builder API for validating sets of files
with nested logical constraints.

It supports:
- required terms (`AND`)
- alternatives (`OR`)
- nested groups
- build-time prevention of duplicate file terms anywhere in the expression tree

## Example

```rust
use file_requirements::FileRequirementBuilder;

let index_base = std::path::Path::new("gencode_pc_v44_index");

let mut b = FileRequirementBuilder::new();
b.require_file(index_base.with_extension("ctab"))?;
b.require_file(index_base.with_extension("ectab"))?;
b.require_file(index_base.with_extension("poison"))?;
b.require_file(index_base.with_extension("poison.json"))?;
b.require_file(index_base.with_extension("refinfo"))?;
b.require_file(index_base.with_extension("sigs.json"))?;

b.require_any(|any| {
    any.require_file(index_base.with_extension("sshash"))?;
    any.require_all(|all| {
        all.require_file(index_base.with_extension("ssi"))?;
        all.require_file(index_base.with_extension("ssi.mphf"))?;
        Ok(())
    })?;
    Ok(())
})?;

b.build().check()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```
