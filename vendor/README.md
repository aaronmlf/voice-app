# Linux accessibility dependency patches

Voice App carries these patches while the published Unix AccessKit adapter still
requires zbus 5. The GUI still uses AccessKit. The Linux adapter and AT-SPI
types and proxies use zbus 6 and keep the AT-SPI interface and event signatures.
The Windows accessibility adapter uses the published crates from crates.io.

| Crate | Upstream revision | SHA-256 of published crate |
| --- | --- | --- |
| `accesskit_unix` 0.24.1 | [AccessKit `f302622`](https://github.com/AccessKit/accesskit/tree/f3026222559c7f5f46d8f25cb7404de68eb149f0/adapters/unix) | `30d5192871228c54f1b0fb4a85cef000e11a3804f25d2165f14afa0daa67f3f6` |
| `accesskit_atspi_common` 0.21.1 | [AccessKit `f302622`](https://github.com/AccessKit/accesskit/tree/f3026222559c7f5f46d8f25cb7404de68eb149f0/adapters/atspi-common) | `24f5baf62e34a4a6c30e3d697e4782f821a7337049c44c3b40576a27dfbb580d` |
| `atspi-common` 0.13.0 | [AT-SPI `f486755`](https://github.com/odilia-app/atspi/tree/f486755b9bf993613f63a779133aafc54925004a/atspi-common) | `20c5617155740c98003016429ad13fe43ce7a77b007479350a9f8bf95a29f63d` |
| `atspi-proxies` 0.13.0 | [AT-SPI `f486755`](https://github.com/odilia-app/atspi/tree/f486755b9bf993613f63a779133aafc54925004a/atspi-proxies) | `2230e48787ed3eb4088996eab66a32ca20c0b67bbd4fd6cdfe79f04f1f04c9fc` |

The crates retain their upstream licenses. The two AccessKit licenses come from
the root of the same upstream revision. The published manifests were adapted for
the local paths and zbus 6 features. AccessKit drives both D-Bus connections on
its accessibility thread through the runtime provided by zbus. The AT-SPI
wrappers retain their wire types, with the XML signature checks run as tests
following [AT-SPI #377](https://github.com/odilia-app/atspi/pull/377).

The test-only `zbus-lockstep` and `zbus-lockstep-macros` come from
[lockstep #10, revision `19169aa`](https://github.com/TTWNO/zbus-lockstep/tree/19169aaa4f67129caef7654c0f3e1c26cea2b830).
They use zbus and zbus_xml 6. The XML fixtures come from `atspi-common` above.

Remove the patches when released AccessKit and AT-SPI versions resolve with
zbus 6 and pass the application's dependency and accessibility checks.
