use crate::js_engine::JsEngine;
use std::collections::HashMap;

/// Bridge between the JavaScript runtime and the HTML document.
///
/// This module provides DOM manipulation capabilities to the JS engine,
/// allowing scripts to modify the HTML structure. After JS execution,
/// the virtual DOM state is serialized back to HTML.
#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
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

    /// Returns the JavaScript code that defines the DOM API shim.
    /// This should be injected into the JS engine before executing page scripts.
    pub fn dom_api_code() -> &'static str {
        r#"
// ===== DOM Bridge Shim =====
// Provides document, window, and element APIs for JavaScript execution.
// After scripts run, call __serializeDOM() to get the final HTML.

var __dom = {
    _elements: new Map(),
    _nextId: 1,
    _document: null,
    _pendingOps: [],
    _rootElement: null
};

// ===== Helper to define property on element =====
function __defProp(el, name, getter, setter) {
    var config = {
        enumerable: true,
        configurable: true,
        get: getter
    };
    if (setter) {
        config.set = setter;
    }
    Object.defineProperty(el, name, config);
}

// ===== Element creation =====
function __createElement(tagName) {
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
    __setupElement(el);
    return el;
}

function __createTextNode(text) {
    var id = '__text_' + (__dom._nextId++);
    var el = {
        _id: id,
        _tag: '#text',
        _textContent: String(text),
        _parent: null
    };
    __dom._elements.set(id, el);
    return el;
}

// ===== Element prototype methods =====
function __setupElement(el) {
    __defProp(el, 'innerHTML',
        function() { return this._innerHTML; },
        function(val) {
            this._innerHTML = val;
            __dom._pendingOps.push({op: 'setInnerHTML', id: this._id, html: val});
        }
    );

    __defProp(el, 'textContent',
        function() { return this._textContent; },
        function(val) {
            this._textContent = String(val);
            __dom._pendingOps.push({op: 'setTextContent', id: this._id, text: String(val)});
        }
    );

    __defProp(el, 'outerHTML', function() {
        return __serializeElement(el);
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
        __dom._pendingOps.push({op: 'appendChild', parentId: this._id, childId: child._id, childTag: child._tag});
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

    el.querySelector = function(selector) { return __querySelector(this, selector); };
    el.querySelectorAll = function(selector) { return __querySelectorAll(this, selector); };
    el.closest = function(selector) { return __closest(this, selector); };

    __defProp(el, 'parentElement', function() { return this._parent; });
    __defProp(el, 'children', function() { return this._children.slice(); });

    __defProp(el, 'nextSibling', function() {
        if (!this._parent) return null;
        var idx = this._parent._children.indexOf(this);
        if (idx === -1 || idx >= this._parent._children.length - 1) return null;
        return this._parent._children[idx + 1];
    });
    __defProp(el, 'previousSibling', function() {
        if (!this._parent) return null;
        var idx = this._parent._children.indexOf(this);
        if (idx <= 0) return null;
        return this._parent._children[idx - 1];
    });
    __defProp(el, 'firstChild', function() { return this._children[0] || null; });
    __defProp(el, 'lastChild', function() { return this._children[this._children.length - 1] || null; });

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
            if (idx > -1) { this._classes.splice(idx, 1); }
            else { this._classes.push(cls); }
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

// ===== Document object =====
__dom._document = {
    _id: '__document__',
    _tag: '#document',
    _children: [],
    createElement: function(tagName) { return __createElement(tagName); },
    createTextNode: function(text) { return __createTextNode(text); },
    getElementById: function(id) { return __querySelector(__dom._rootElement, '#' + id); },
    getElementsByClassName: function(cls) { return __querySelectorAll(__dom._rootElement, '.' + cls); },
    getElementsByTagName: function(tag) { return __querySelectorAll(__dom._rootElement, tag); },
    querySelector: function(selector) { return __querySelector(__dom._rootElement, selector); },
    querySelectorAll: function(selector) { return __querySelectorAll(__dom._rootElement, selector); },
    addEventListener: function(type, handler) {
        if (!this._events) this._events = {};
        if (!this._events[type]) this._events[type] = [];
        this._events[type].push(handler);
    }
};

__defProp(__dom._document, 'documentElement', function() { return __dom._rootElement; });
__defProp(__dom._document, 'body', function() { 
    if (__dom._rootElement && __dom._rootElement._children) {
        for (var i = 0; i < __dom._rootElement._children.length; i++) {
            if (__dom._rootElement._children[i]._tag === 'body') {
                return __dom._rootElement._children[i];
            }
        }
    }
    return null;
});
__defProp(__dom._document, 'head', function() {
    if (__dom._rootElement && __dom._rootElement._children) {
        for (var i = 0; i < __dom._rootElement._children.length; i++) {
            if (__dom._rootElement._children[i]._tag === 'head') {
                return __dom._rootElement._children[i];
            }
        }
    }
    return null;
});

// Set document and window as globals using direct property assignment
this.document = __dom._document;
this.window = this;
this.window.document = this.document;

// ===== HTML Parser: inject HTML string into virtual DOM =====
function __injectHTML(htmlStr) {
    // Clear existing elements
    __dom._elements = new Map();
    __dom._pendingOps = [];
    __dom._nextId = 1;

    // Create html element as root
    var htmlEl = __createElement('html');
    __dom._rootElement = htmlEl;
    if (__dom._document) {
        __dom._document._children = [htmlEl];
        htmlEl._parent = __dom._document;
    }

    // Create head and body
    var headEl = __createElement('head');
    var bodyEl = __createElement('body');
    htmlEl._children = [headEl, bodyEl];
    headEl._parent = htmlEl;
    bodyEl._parent = htmlEl;

    // Extract body content using simple regex
    var bodyMatch = htmlStr.match(/<body[^>]*>([\s\S]*?)<\/body>/i);
    var bodyContent = '';
    if (bodyMatch) {
        bodyContent = bodyMatch[1];
    } else {
        bodyContent = htmlStr;
    }
    
    // Parse body HTML into virtual DOM elements
    __parseBodyContent(bodyEl, bodyContent);
}

// Parse simple HTML into virtual DOM elements
// This handles: <tag>, <tag attr="val">, </tag>, text nodes
// It doesn't handle all edge cases but is sufficient for common scenarios
function __parseBodyContent(parentEl, htmlStr) {
    var remaining = htmlStr;
    var tagRegex = /<\/?([a-zA-Z0-9_-]+)([^>]*)>/g;
    var lastEnd = 0;
    var stack = [parentEl];
    
    while (tagRegex.lastIndex < remaining.length) {
        var match = tagRegex.exec(remaining);
        if (!match) break;
        
        var tagStart = match.index;
        var tagEnd = tagRegex.lastIndex;
        
        // Text content before this tag
        if (tagStart > lastEnd) {
            var text = remaining.substring(lastEnd, tagStart).trim();
            if (text) {
                var textEl = __createTextNode(text);
                var current = stack[stack.length - 1];
                if (current) {
                    current._children.push(textEl);
                    textEl._parent = current;
                }
            }
        }
        
        var fullTag = match[0];
        var tagName = match[1].toLowerCase();
        var attrsStr = match[2];
        
        // Check if it's a closing tag
        if (fullTag.indexOf('</') === 0) {
            // Closing tag - pop from stack
            if (stack.length > 1) {
                stack.pop();
            }
            lastEnd = tagEnd;
            continue;
        }
        
        // Self-closing tags
        var voidElements = {
            area: true, base: true, br: true, col: true, embed: true,
            hr: true, img: true, input: true, link: true, meta: true,
            param: true, source: true, track: true, wbr: true
        };
        var isSelfClosing = voidElements[tagName] || fullTag.endsWith('/>');
        
        // Create element
        var el = __createElement(tagName);
        
        // Parse attributes
        var attrRegex = /([a-zA-Z_:][a-zA-Z0-9_:.-]*)\s*=\s*"([^"]*)"/g;
        var attrMatch;
        while ((attrMatch = attrRegex.exec(attrsStr)) !== null) {
            el._attributes[attrMatch[1]] = attrMatch[2];
        }
        // Handle single-quoted attributes
        var attrRegex2 = /([a-zA-Z_:][a-zA-Z0-9_:.-]*)\s*=\s*'([^']*)'/g;
        while ((attrMatch = attrRegex2.exec(attrsStr)) !== null) {
            el._attributes[attrMatch[1]] = attrMatch[2];
        }
        // Handle boolean attributes (no value)
        var boolAttrRegex = /\s+([a-zA-Z_:][a-zA-Z0-9_:.-]*)(?:\s*=\s*\S+)?/g;
        // Only for attributes without =
        var attrParts = attrsStr.trim().split(/\s+/);
        for (var a = 0; a < attrParts.length; a++) {
            var part = attrParts[a];
            if (part && part.indexOf('=') === -1 && !part.startsWith('/')) {
                el._attributes[part] = '';
            }
        }
        
        // Add to parent
        var current = stack[stack.length - 1];
        if (current) {
            current._children.push(el);
            el._parent = current;
        }
        
        if (!isSelfClosing) {
            stack.push(el);
        }
        
        lastEnd = tagEnd;
    }
    
    // Remaining text after last tag
    if (lastEnd < remaining.length) {
        var text = remaining.substring(lastEnd).trim();
        if (text) {
            var textEl = __createTextNode(text);
            var current = stack[stack.length - 1];
            if (current) {
                current._children.push(textEl);
                textEl._parent = current;
            }
        }
    }
}

// ===== HTML Serializer: convert virtual DOM element to HTML =====
function __serializeElement(el) {
    if (!el) return '';
    if (el._tag === '#document') {
        return __serializeElement(__dom._rootElement);
    }
    // Text nodes: just return their text content directly, not as HTML tags
    if (el._tag === '#text') {
        return el._textContent;
    }

    var tag = el._tag;
    var attrs = '';
    for (var name in el._attributes) {
        if (el._attributes.hasOwnProperty(name)) {
            var val = el._attributes[name];
            attrs += ' ' + name + '="' + val.replace(/"/g, '&quot;') + '"';
        }
    }

    // Self-closing tags
    var voidElements = {
        area: true, base: true, br: true, col: true, embed: true,
        hr: true, img: true, input: true, link: true, meta: true,
        param: true, source: true, track: true, wbr: true
    };

    if (voidElements[tag]) {
        return '<' + tag + attrs + '>';
    }

    var innerHTML = '';
    // Text content takes priority - if explicitly set, use it
    if (el._textContent && el._textContent.length > 0) {
        innerHTML = el._textContent;
    } else if (el._innerHTML && el._innerHTML.length > 0) {
        innerHTML = el._innerHTML;
    } else if (el._children && el._children.length > 0) {
        for (var i = 0; i < el._children.length; i++) {
            innerHTML += __serializeElement(el._children[i]);
        }
    }

    return '<' + tag + attrs + '>' + innerHTML + '</' + tag + '>';
}

// ===== Serialize full DOM to HTML string =====
function __serializeDOM() {
    if (__dom._rootElement) {
        return '<!DOCTYPE html>\n' + __serializeElement(__dom._rootElement);
    }
    return '';
}

// ===== Query selector helpers =====
function __matchSelector(el, selector) {
    if (!el || !el._tag) return false;
    // ID selector
    if (selector.startsWith('#')) {
        var id = selector.substring(1);
        return el._attributes && el._attributes['id'] === id;
    }
    // Class selector
    if (selector.startsWith('.')) {
        var cls = selector.substring(1);
        var classes = (el._attributes && el._attributes['class'] || '').split(/\s+/);
        return classes.indexOf(cls) !== -1;
    }
    // Tag name selector (case-insensitive for HTML)
    return el._tag && el._tag.toLowerCase() === selector.toLowerCase();
}

function __querySelector(root, selector) {
    if (!root || !selector) return null;
    selector = selector.trim();
    var children = root._children || [];
    for (var i = 0; i < children.length; i++) {
        var child = children[i];
        if (__matchSelector(child, selector)) {
            return child;
        }
        var found = __querySelector(child, selector);
        if (found) return found;
    }
    return null;
}

function __querySelectorAll(root, selector) {
    var results = [];
    if (!root || !selector) return results;
    selector = selector.trim();
    var children = root._children || [];
    for (var i = 0; i < children.length; i++) {
        var child = children[i];
        if (__matchSelector(child, selector)) {
            results.push(child);
        }
        var found = __querySelectorAll(child, selector);
        results = results.concat(found);
    }
    return results;
}

function __closest(el, selector) {
    if (!el || !selector) return null;
    selector = selector.trim();
    var current = el;
    while (current) {
        if (__matchSelector(current, selector)) return current;
        current = current._parent;
    }
    return null;
}

// ===== MutationObserver stub =====
var MutationObserver = function(callback) {
    this._callback = callback;
    this._observe = function(target, options) {
        if (!__dom._observers) __dom._observers = [];
        __dom._observers.push(this);
    };
    this.disconnect = function() {};
    this.takeRecords = function() { return []; };
};

// Fire DOMContentLoaded
if (typeof document !== 'undefined' && document.addEventListener) {
    document.addEventListener('DOMContentLoaded', function() {});
}
"#
    }

    /// Inject the page HTML into the JS engine's virtual DOM.
    pub fn inject_html_into_js(&self, engine: &mut JsEngine, html: &str) -> Result<(), String> {
        // Escape the HTML string for use in JavaScript
        let escaped = html
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\'', "\\'")
            .replace('\t', "\\t");
        let js_code = format!("__injectHTML('{}');", escaped);
        engine.execute(&js_code, "__injectHTML")
    }

    /// Serialize the JS virtual DOM back to HTML.
    pub fn serialize_dom_from_js(&self, engine: &mut JsEngine) -> Result<String, String> {
        engine.eval_expression("__serializeDOM()")
    }

    /// Get the current HTML content.
    pub fn html(&self) -> &str {
        &self.html
    }

    /// Set the HTML content.
    pub fn set_html(&mut self, html: String) {
        self.html = html;
    }

    /// Finalize: serialize the JS virtual DOM to HTML and update self.html.
    pub fn finalize(&mut self, engine: &mut JsEngine) -> String {
        match self.serialize_dom_from_js(engine) {
            Ok(serialized) => {
                self.html = serialized;
                self.html.clone()
            }
            Err(_) => self.html.clone()
        }
    }

    /// Process pending DOM operations (legacy, kept for compatibility).
    pub fn process_pending_operations(&mut self) -> String {
        self.pending_changes.clear();
        self.html.clone()
    }

    /// Clear all pending changes.
    pub fn clear_pending(&mut self) {
        self.pending_changes.clear();
    }

    /// Get the number of pending DOM changes.
    pub fn pending_count(&self) -> usize {
        self.pending_changes.len()
    }
}
