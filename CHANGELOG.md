Changelog
=========

Version 0.2.0
-------------

To be released.

 -  Changed NecrassRs releases to use one shared version tag and one set of
    product-level release notes for all eight public packages. This release
    changes release coordination only; applications upgrading from 0.1.2 do
    not require source changes.  [[#86]]

[#86]: https://github.com/Necrass-Dev/NecrassRs/issues/86


Version 0.1.2
-------------

Released on November 4, 2026.

 -  Added GraphQL subscription execution, including generated subscriber
    contracts, source streams, response delivery, and resolver source
    synchronization.
 -  Made generated enum values cloneable so owned values can be delivered by
    subscription streams.
 -  Fixed validation of merged subscription root selections and their response
    keys.


Version 0.1.1
-------------

Released on November 2, 2026.

 -  Documented the public Reed Parser, Reed Compiler, and NecrassRs package
    boundaries and removed obsolete Apollo migration records.


Version 0.1.0
-------------

Initial release.  Released on November 2, 2026.

 -  Added the Reed GraphQL parser and compiler packages used by NecrassRs.
 -  Added SDL-first GraphQL query and mutation execution with generated Rust
    contracts, editable resolver synchronization, custom scalar support,
    composite results, input coercion, error propagation, and concurrent query
    fields.
 -  Added Axum and Actix Web adapters with GraphQL-over-HTTP negotiation,
    request context isolation, introspection, and GraphiQL support.
 -  Added the `necrass` initializer with Axum and Actix Web project templates.
