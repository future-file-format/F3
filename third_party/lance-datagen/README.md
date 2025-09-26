Vendored from commit c0e1f158609628f08239520596d90ba3d8c0477f so that we do not need to deal with Arrow version mismatch.

To accommodate arrow-rs 53.0 changes, we change `Buffer:from` to `Buffer::from_vec` in some code.