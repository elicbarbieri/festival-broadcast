# External libraries (with patches)

Some external libraries that with some custom patches for some features `Festival` needs.

| Fork branch | Purpose |
|-------------|---------|
| `festival`  | The main patched fork that `Festival` uses
| `master`    | Up-to-date, un-modified upstream branch


# Library Patches

## Symphonia

* Add Metadata.into_current() & Metadata.into_latest()
  * uses Vec.pop_front() to return owned data, instead of iterating through all metadata revisions
* Add MetadataRevision.into_inner()
  * returns the inner data of MetadataRevision as owned, to prevent cloning image & text data when assembling a collection


