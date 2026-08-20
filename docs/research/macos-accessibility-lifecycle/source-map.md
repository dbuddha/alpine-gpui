# macOS accessibility source map

| Source | Pin | Evidence use |
| --- | --- | --- |
| [Apple custom control integration](https://developer.apple.com/documentation/accessibility/integrating-accessibility-into-your-app) | Current official documentation at research time | Platform integration contract |
| [NSAccessibility protocol](https://developer.apple.com/documentation/appkit/nsaccessibilityprotocol) | Current official documentation at research time | Roles, attributes, actions, and focus behavior |
| [NSTextInputContext marked-text discard](https://developer.apple.com/documentation/appkit/nstextinputcontext/discardmarkedtext%28%29) | Current official documentation at research time | Native IME cancellation boundary |
| [Announcement notification](https://developer.apple.com/documentation/appkit/nsaccessibilityannouncementrequestednotification) | Current official documentation at research time | Announcement payload path |
| [AXObserverCreate](https://developer.apple.com/documentation/applicationservices/1460133-axobservercreate) | Current official documentation at research time | External observation mechanism |
| [Accessibility Inspector](https://developer.apple.com/documentation/accessibility/accessibility-inspector) | Current official documentation at research time | Trusted inspection lane |
| [Zed accessibility state](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/window/a11y.rs) | `e17dc4f9d50db73a458b64dcce50ecd4878b98a3` | Pinned comparator mechanism |
| [Zed external accessibility test](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/nix/tests/a11y_atspi_test.py) | `e17dc4f9d50db73a458b64dcce50ecd4878b98a3` | External observation precedent |
| [AccessKit macOS adapter](https://github.com/AccessKit/accesskit/blob/2dfdd7b92e68edd4276841a5061f31ffc77e718b/adapters/macos/src/adapter.rs) | `2dfdd7b92e68edd4276841a5061f31ffc77e718b` | Focus forwarding reference |
| [AccessKit notification path](https://github.com/AccessKit/accesskit/blob/2dfdd7b92e68edd4276841a5061f31ffc77e718b/adapters/macos/src/event.rs#L46-L129) | `2dfdd7b92e68edd4276841a5061f31ffc77e718b` | Notification mechanism reference |
| [AccessKit destruction path](https://github.com/AccessKit/accesskit/blob/2dfdd7b92e68edd4276841a5061f31ffc77e718b/adapters/macos/src/context.rs#L91-L102) | `2dfdd7b92e68edd4276841a5061f31ffc77e718b` | Removed-element lifetime reference |
| [GitHub-hosted runners](https://docs.github.com/en/actions/concepts/runners/github-hosted-runners) | Current official documentation at research time | Hosted environment claim limits |

Alpine observations use source revision
`53bf751deff87d26811bd1d66fa6fb0d375f53d2`. Mechanism references are not
qualification evidence and no upstream source is copied.
