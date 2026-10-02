import { parse } from "@babel/parser";
import traverseModule from "@babel/traverse";
import { createHash } from "node:crypto";

const traverse = traverseModule.default ?? traverseModule;
const protocolShapes = new Set([
  "5bf64da3b8386af46a6fb2c2d8829a507130ba9896237a813b8e04c4ce8eb515",
  "eae1b49427aebf3ed3d1119de1c303125c4f78b3b0ca644f055ed07ec2c2be30",
]);

function requireContract(condition, message) {
  if (!condition) throw new Error(message);
}

function member(node, property, object = "ThisExpression") {
  return node?.type === "MemberExpression" && !node.computed &&
    node.object.type === object && node.property.name === property;
}

function shape(path) {
  const bindings = new Map();
  const names = new WeakMap();
  path.traverse({
    Identifier(p) {
      if (!p.isBindingIdentifier() && !p.isReferencedIdentifier()) return;
      const binding = p.scope.getBinding(p.node.name);
      if (!binding) return;
      if (!bindings.has(binding)) bindings.set(binding, `local${bindings.size}`);
      names.set(p.node, bindings.get(binding));
    },
  });
  // Positions, formatting and local variable names are not protocol behavior.
  const clean = JSON.stringify(path.node, function (key, value) {
    if (["start", "end", "loc", "extra", "leadingComments",
      "trailingComments", "innerComments"].includes(key)) return undefined;
    if (key === "name" && names.has(this)) return names.get(this);
    return value;
  });
  return createHash("sha256").update(clean).digest("hex");
}

function boundIdentifier(path, binding) {
  return path.isIdentifier() && path.scope.getBinding(path.node.name) === binding;
}

function adaptiveRequest(send) {
  requireContract(send.node.async && send.node.params.length >= 2 &&
    send.node.params.slice(0, 2).every(p => p.type === "Identifier"),
  "Changed browser request signature");
  const command = send.scope.getBinding(send.node.params[0].name);
  const payload = send.scope.getBinding(send.node.params[1].name);
  requireContract(command?.constant && payload?.constant, "Rebound request arguments");
  let session, header;
  let callbacks = 0, forwarding = 0, headerWrites = 0;
  send.traverse({
    VariableDeclarator(p) {
      const init = p.get("init");
      if (init.isCallExpression() && member(init.node.callee, "getSessionParams")) {
        requireContract(!session && p.get("id").isIdentifier() &&
          init.node.arguments.length === 0, "Ambiguous session parameters");
        session = p.scope.getBinding(p.node.id.name);
      }
    },
  });
  requireContract(session?.constant, "Missing stable session parameters");
  send.traverse({
    CallExpression(p) {
      if (member(p.node.callee, "readRequestHeaderEnabled")) {
        callbacks++;
        requireContract(p.node.arguments.length === 0 &&
          p.parentPath.isAwaitExpression(), "Changed policy invocation");
        const declaration = p.findParent(q => q.isVariableDeclarator());
        requireContract(declaration?.get("id").isIdentifier(), "Missing header decision");
        header = declaration.scope.getBinding(declaration.node.id.name);
        requireContract(header?.constant, "Rebound header decision");
      }
      if (member(p.node.callee, "sendRequest")) {
        forwarding++;
        const args = p.get("arguments");
        requireContract(args.length === 2 && boundIdentifier(args[0], command) &&
          args[1].isObjectExpression(), "Changed browser request forwarding");
        const spreads = args[1].get("properties").filter(q => q.isSpreadElement())
          .map(q => q.get("argument"));
        const properties = args[1].get("properties");
        requireContract(properties.length === 2 && spreads.length === 2 &&
          boundIdentifier(spreads[0], payload) && boundIdentifier(spreads[1], session),
        "Missing request or session parameters");
      }
    },
  });
  send.traverse({
    AssignmentExpression(p) {
      const left = p.get("left");
      if (left.isMemberExpression() && !left.node.computed &&
        left.node.property.name === "agent_request_header_enabled" &&
        boundIdentifier(left.get("object"), session)) {
        headerWrites++;
        requireContract(header && boundIdentifier(p.get("right"), header),
          "Changed request identification data flow");
      }
    },
  });
  requireContract(callbacks === 1 && forwarding === 1 && headerWrites === 1,
    "Ambiguous request identification contract");
}

function requestMethod(methods) {
  const info = methods.filter(p => p.isClassMethod() && !p.node.computed &&
    p.node.key.name === "getInfo");
  requireContract(info.length === 1 && info[0].node.async, "Ambiguous browser handshake");
  let methodName, result, calls = 0, stores = 0;
  info[0].traverse({
    VariableDeclarator(p) {
      const awaited = p.get("init");
      if (!awaited.isAwaitExpression()) return;
      const call = awaited.get("argument");
      if (!call.isCallExpression() || call.node.arguments[0]?.value !== "getInfo") return;
      requireContract(call.node.callee.type === "MemberExpression" &&
        !call.node.callee.computed && call.node.callee.object.type === "ThisExpression" &&
        p.get("id").isIdentifier(), "Changed browser handshake request");
      calls++;
      methodName = call.node.callee.property.name;
      result = p.scope.getBinding(p.node.id.name);
    },
  });
  info[0].traverse({
    AssignmentExpression(p) {
      if (member(p.node.left, "clientInfo")) {
        stores++;
        requireContract(result && boundIdentifier(p.get("right"), result),
          "Changed browser handshake result");
      }
    },
  });
  requireContract(calls === 1 && stores === 1 && result?.constant,
    "Changed browser client handshake");
  const send = methods.filter(p => p.isClassMethod() && !p.node.computed &&
    p.node.key.name === methodName);
  requireContract(send.length === 1, "Ambiguous browser request method");
  return send[0];
}

export function inspectEntry(source) {
  requireContract(typeof source === "string" &&
    Buffer.byteLength(source) <= 1024 * 1024, "Invalid entry size");
  const ast = parse(source, { sourceType: "module" });
  let imported, launches = 0;
  traverse(ast, {
    ImportDeclaration(p) {
      if (p.node.source.value !== "@oai/cua-repl") return;
      requireContract(!imported && p.node.specifiers.length === 1 &&
        p.get("specifiers.0").isImportNamespaceSpecifier(), "Ambiguous CUA entry import");
      imported = p.scope.getBinding(p.node.specifiers[0].local.name);
    },
  });
  traverse(ast, {
    CallExpression(p) {
      const callee = p.get("callee");
      if (callee.isMemberExpression() && !callee.node.computed &&
        callee.node.property.name === "launch" &&
        boundIdentifier(callee.get("object"), imported)) {
        launches++;
        requireContract(p.node.arguments.length === 0 && p.parentPath.isAwaitExpression(),
          "Changed CUA launch signature");
      }
    },
  });
  requireContract(imported?.constant && launches === 1, "Missing stable CUA launch entry");
}

export function inspectService(source) {
  requireContract(typeof source === "string" &&
    Buffer.byteLength(source) <= 32 * 1024 * 1024, "Invalid service size");
  const ast = parse(source, { sourceType: "module" });
  const reserved = new Set(["cppNativeIdentificationReader"]);
  const candidates = [];
  traverse(ast, {
    Identifier(path) {
      requireContract(!reserved.has(path.node.name), "Conflicting adapter");
    },
    NewExpression(path) {
      const args = path.get("arguments");
      if (args.length !== 5 || !member(args[3].node, "turnEndedTracker")) return;
      requireContract(path.get("callee").isIdentifier(), "Unsupported constructor");
      const binding = path.scope.getBinding(path.node.callee.name);
      requireContract(binding?.constant && binding.path.isVariableDeclarator() &&
        binding.path.get("init").isClassExpression(), "Unstable browser constructor");
      const cls = binding.path.get("init");
      const methods = cls.get("body.body");
      const ctor = methods.find(p => p.node.kind === "constructor");
      requireContract(ctor && ctor.node.params.length === 5 &&
        ctor.node.params.every(p => p.type === "Identifier"), "Unsupported constructor arguments");
      const assigned = new Map();
      let handlerBound = false;
      ctor.traverse({
        AssignmentExpression(p) {
          const { left, right } = p.node;
          if (left.type === "MemberExpression" && left.object.type === "ThisExpression" &&
            !left.computed && right.type === "Identifier") {
            requireContract(!assigned.has(left.property.name), "Duplicate constructor field");
            assigned.set(left.property.name,
              ctor.node.params.findIndex(x => ctor.scope.getBinding(x.name) ===
                p.scope.getBinding(right.name)));
          }
        },
        CallExpression(p) {
          if (member(p.node.callee, "registerRequestHandlerObject") &&
            p.node.arguments.length === 1 &&
            p.node.arguments[0].name === ctor.node.params[1].name) handlerBound = true;
        },
      });
      requireContract(handlerBound && assigned.get("apiTransport") === 0 &&
        assigned.get("getTurnMetadata") === 2 && assigned.get("turnEndedTracker") === 3 &&
        assigned.get("readRequestHeaderEnabled") === 4, "Changed callback contract");
      requireContract(member(args[1].node, "clientApi") &&
        args[2].isArrowFunctionExpression() && args[2].node.params.length === 0,
      "Changed browser arguments");
      const call = args[2].get("body");
      requireContract(call.isCallExpression() && call.get("callee").isIdentifier() &&
        call.node.arguments.length === 1 && member(call.node.arguments[0], "runtime") &&
        args[4].isIdentifier(), "Changed callback arguments");
      const metadata = call.node.callee.name;
      const policy = args[4].node.name;
      const metaBinding = call.scope.getBinding(metadata);
      const policyBinding = path.scope.getBinding(policy);
      requireContract(metaBinding?.constant && metaBinding.path.isFunctionDeclaration() &&
        policyBinding?.constant && policyBinding.path.isFunctionDeclaration(),
      "Unstable callback bindings");
      const literals = p => {
        const values = [];
        p.traverse({ StringLiteral(q) { values.push(q.node.value); } });
        return values;
      };
      requireContract(literals(metaBinding.path).includes("x-codex-turn-metadata") &&
        metaBinding.path.node.params.length === 1 &&
        policyBinding.path.node.async && policyBinding.path.node.params.length === 0 &&
        literals(policyBinding.path).includes("codex_browser_use_agent_request_header"),
      "Changed metadata or policy protocol");
      requireContract(shape(metaBinding.path) ===
        "96146f3d4c51e5707a7cb38783b06294b1f60e9bbc1f0b0cba93c63d23222ce8" &&
        shape(policyBinding.path) ===
        "3c64bbeed6876be6468d1700b8f63c217b458ac1f495856ede7e7f939c9b3df2",
      "Changed metadata or policy semantics");
      const send = requestMethod(methods);
      const contractSha = shape(send);
      // Known shapes are a fast path, never the only admission path.
      if (!protocolShapes.has(contractSha)) adaptiveRequest(send);
      // Reject shadowing at the actual insertion site, not just the declaration.
      requireContract(path.scope.getBinding(metadata) === metaBinding &&
        !path.scope.getBinding("cppNativeIdentificationReader"), "Shadowed metadata binding");
      candidates.push({
        start: Buffer.byteLength(source.slice(0, args[4].node.start)),
        end: Buffer.byteLength(source.slice(0, args[4].node.end)),
        policy, metadata, contractSha,
      });
    },
  });
  requireContract(candidates.length === 1, "Expected one browser callback contract");
  return candidates[0];
}

if (process.argv.includes("--stdin")) {
  let source = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", chunk => {
    source += chunk;
    // JSON escaping can expand each source byte sixfold; component limits are checked after decoding.
    if (Buffer.byteLength(source) > 200 * 1024 * 1024) process.exit(2);
  });
  process.stdin.on("end", () => {
    try {
      const request = JSON.parse(source);
      inspectEntry(request.entry);
      console.log(JSON.stringify(inspectService(request.service)));
    } catch {
      // Never echo service source, parser excerpts or local paths.
      console.error("Browser service does not match the supported semantic contract.");
      process.exitCode = 2;
    }
  });
}
