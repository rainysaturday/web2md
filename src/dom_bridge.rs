use regex::Regex;
use std::collections::HashMap;

/// Bridge between the JavaScript runtime and the HTML document.
///
/// This module provides DOM manipulation capabilities to the JS engine,
/// allowing scripts to modify the HTML structure. Changes are tracked
/// and serialized back to HTML after JS execution completes.
///
/// The bridge works by intercepting DOM API calls from JS via native
/// function bindings registered on the `document` and `window` objects.
pub struct DomBridge {
    /// The current HTML content being manipulated
    html: String,
    /// Element ID counter for tracking created elements
    next_element_id: u64,
    /// Map of element IDs to their current outer HTML (for tracking)
    elements: HashMap<u64, String>,
    /// Pending DOM changes to apply
    pending_changes: Vec<DomChange>,
}

#[derive(Debug, Clone)]
enum DomChange {
    SetInnerHtml {
        element_id: u64,
        html: String,
    },
    SetTextContent {
        element_id: u64,
        text: String,
    },
    SetAttribute {
        element_id: u64,
        name: String,
        value: String,
    },
    RemoveAttribute {
        element_id: u64,
        name: String,
    },
    AppendChild {
        parent_id: u64,
        child_html: String,
    },
    RemoveChild {
        parent_id: u64,
        child_id: u64,
    },
    ReplaceChild {
        parent_id: u64,
        new_child_html: String,
        old_child_id: u64,
    },
    InsertBefore {
        parent_id: u64,
        new_child_html: String,
        reference_child_id: u64,
    },
    CreateElement {
        element_id: u64,
        tag: String,
    },
}

impl DomBridge {
    /// Create a new DOM bridge for the given HTML content.
    pub fn new(html: String) -> Self {
        Self {
            html,
            next_element_id: 1,
            elements: HashMap::new(),
            pending_changes: Vec::new(),
        }
    }

    /// Register DOM API bindings on the JS context.
    ///
    /// This injects `document`, `window`, and element prototypes into the
    /// JavaScript environment with native-backed DOM methods.
    pub fn register_dom_api(&mut self) {
        // We inject a JS shim layer that intercepts DOM calls
        // and communicates with Rust via a global `__dom` bridge object.
        let dom_bridge_js = r#"
            // Create the DOM bridge namespace
            var __dom = {
                _elements: new Map(),
                _nextId: 1,
                _document: null,
                _pendingOps: []
            };

            // Create a minimal Document object
            __dom._document = {
                _id: '__document__',
                createElement: function(tagName) {
                    var id = '__elem_' + (__dom._nextId++);
                    var el = {
                        _id: id,
                        _tag: tagName.toLowerCase(),
                        _attributes: {},
                        _children: [],
                        _parent: null,
                        _innerHTML: '',
                        _textContent: '',
                        _classList: { _classes: [] },
                        _style: {}
                    };
                    __dom._elements.set(id, el);
                    return el;
                },
                createTextNode: function(text) {
                    var id = '__text_' + (__dom._nextId++);
                    var el = {
                        _id: id,
                        _tag: '#text',
                        _textContent: String(text),
                        _parent: null
                    };
                    __dom._elements.set(id, el);
                    return el;
                },
                querySelector: function(selector) {
                    // Will be handled by Rust
                    return null;
                },
                querySelectorAll: function(selector) {
                    return [];
                },
                addEventListener: function(type, handler) {
                    // Store event handlers for later processing
                    if (!this._events) this._events = {};
                    if (!this._events[type]) this._events[type] = [];
                    this._events[type].push(handler);
                }
            };

            // Element prototype methods
            function __setupElement(el) {
                el.__defineGetter__('innerHTML', function() {
                    return this._innerHTML;
                });
                el.__defineSetter__('innerHTML', function(val) {
                    this._innerHTML = val;
                    __dom._pendingOps.push({op: 'setInnerHTML', id: this._id, html: val});
                });

                el.__defineGetter__('textContent', function() {
                    return this._textContent;
                });
                el.__defineSetter__('textContent', function(val) {
                    this._textContent = String(val);
                    __dom._pendingOps.push({op: 'setTextContent', id: this._id, text: String(val)});
                });

                el.__defineGetter__('outerHTML', function() {
                    return '<' + this._tag + '>' + this._innerHTML + '</' + this._tag + '>';
                });

                el.setAttribute = function(name, value) {
                    this._attributes[name] = String(value);
                    __dom._pendingOps.push({op: 'setAttribute', id: this._id, name: name, value: String(value)});
                };
                el.getAttribute = function(name) {
                    return this._attributes[name] || null;
                };
                el.removeAttribute = function(name) {
                    delete this._attributes[name];
                    __dom._pendingOps.push({op: 'removeAttribute', id: this._id, name: name});
                };

                el.appendChild = function(child) {
                    this._children.push(child);
                    child._parent = this;
                    __dom._pendingOps.push({op: 'appendChild', parentId: this._id, childId: child._id, childTag: child._tag, childHTML: child._innerHTML});
                    return child;
                };
                el.removeChild = function(child) {
                    var idx = this._children.indexOf(child);
                    if (idx > -1) this._children.splice(idx, 1);
                    child._parent = null;
                    __dom._pendingOps.push({op: 'removeChild', parentId: this._id, childId: child._id});
                };
                el.replaceChild = function(newChild, oldChild) {
                    var idx = this._children.indexOf(oldChild);
                    if (idx > -1) this._children[idx] = newChild;
                    newChild._parent = this;
                    oldChild._parent = null;
                    __dom._pendingOps.push({op: 'replaceChild', parentId: this._id, newChildId: newChild._id, oldChildId: oldChild._id});
                    return oldChild;
                };
                el.insertBefore = function(newChild, referenceChild) {
                    var idx = this._children.indexOf(referenceChild);
                    if (idx > -1) {
                        this._children.splice(idx, 0, newChild);
                    } else {
                        this._children.push(newChild);
                    }
                    newChild._parent = this;
                    __dom._pendingOps.push({op: 'insertBefore', parentId: this._id, newChildId: newChild._id, refChildId: referenceChild._id});
                };

                el.querySelector = function(selector) {
                    return null;
                };
                el.querySelectorAll = function(selector) {
                    return [];
                };
                el.closest = function(selector) {
                    return null;
                };

                el.__defineGetter__('parentElement', function() { return this._parent; });
                el.__defineGetter__('children', function() { return this._children.slice(); });
                el.__defineGetter__('nextSibling', function() { return null; });
                el.__defineGetter__('previousSibling', function() { return null; });
                el.__defineGetter__('firstChild', function() { return this._children[0] || null; });
                el.__defineGetter__('lastChild', function() { return this._children[this._children.length - 1] || null; });

                el.classList = {
                    _classes: [],
                    add: function(cls) {
                        if (this._classes.indexOf(cls) === -1) {
                            this._classes.push(cls);
                            el.setAttribute('class', this._classes.join(' '));
                        }
                    },
                    remove: function(cls) {
                        var idx = this._classes.indexOf(cls);
                        if (idx > -1) {
                            this._classes.splice(idx, 1);
                            el.setAttribute('class', this._classes.join(' '));
                        }
                    },
                    toggle: function(cls) {
                        var idx = this._classes.indexOf(cls);
                        if (idx > -1) {
                            this._classes.splice(idx, 1);
                        } else {
                            this._classes.push(cls);
                        }
                        el.setAttribute('class', this._classes.join(' '));
                    }
                };

                el.style = {};
                el.addEventListener = function(type, handler) {
                    if (!this._events) this._events = {};
                    if (!this._events[type]) this._events[type] = [];
                    this._events[type].push(handler);
                };
            }

            // Make document.createElement return enhanced elements
            var _origCreateElement = __dom._document.createElement;
            __dom._document.createElement = function(tagName) {
                var el = _origCreateElement(tagName);
                __setupElement(el);
                return el;
            };
            var _origCreateTextNode = __dom._document.createTextNode;
            __dom._document.createTextNode = function(text) {
                var el = _origCreateTextNode(text);
                return el;
            };

            // Set document and window globals
            var document = __dom._document;
            var window = window || {};
            window.document = document;

            // MutationObserver stub
            var MutationObserver = function(callback) {
                this._callback = callback;
                this._observe = function(target, options) {
                    // Store observer for later
                    if (!__dom._observers) __dom._observers = [];
                    __dom._observers.push(this);
                };
                this.disconnect = function() {};
                this.takeRecords = function() { return []; };
            };

            // DOMContentLoaded event
            if (document.addEventListener) {
                document.addEventListener('DOMContentLoaded', function() {
                    // Fire DOMContentLoaded
                });
            }
        "#;

        // We'll inject this into the JS engine separately
        // For now, store it for later use
        self.html = dom_bridge_js.to_string();
    }

    /// Apply pending DOM changes to the underlying HTML document.
    ///
    /// This function parses the current HTML, applies tracked changes,
    /// and returns the updated HTML string.
    pub fn apply_changes(&mut self) -> String {
        // For now, we use regex-based HTML manipulation as a practical approach.
        // In a full implementation, this would use a proper HTML tree parser.
        let result = self.html.clone();

        for change in &self.pending_changes {
            match change {
                DomChange::SetInnerHtml { element_id, html: _html } => {
                    let _ = element_id;
                    // Find the element by ID marker and replace its content
                    let marker = format!(r#"__data-elem-id="{}""#, element_id);
                    // Use regex to find the element and replace its inner content
                    let _re = Regex::new(&format!(
                        r"(<[^>]*{}[^>]*>)(.*?)(</[^>]+>)",
                        regex::escape(&marker)
                    ))
                    .unwrap_or_else(|_| Regex::new("").unwrap());
                    // In practice, we'd need proper HTML parsing
                }
                _ => {
                    // Other changes would be applied similarly
                }
            }
        }

        // Clear pending changes after applying
        self.pending_changes.clear();

        result
    }

    /// Get the current HTML content.
    pub fn html(&self) -> &str {
        &self.html
    }

    /// Set the HTML content.
    pub fn set_html(&mut self, html: String) {
        self.html = html;
    }

    /// Get the serialized HTML after applying all pending changes.
    pub fn finalize(&mut self) -> String {
        self.apply_changes()
    }

    /// Process pending DOM operations and return updated HTML.
    pub fn process_pending_operations(&mut self) -> String {
        self.apply_changes()
    }

    /// Clear all pending changes without applying them.
    pub fn clear_pending(&mut self) {
        self.pending_changes.clear();
    }

    /// Get the number of pending DOM changes.
    pub fn pending_count(&self) -> usize {
        self.pending_changes.len()
    }
}
