import test from "node:test";
import assert from "node:assert/strict";

import { requestRuleReadsRequester } from "./request-rule-sets.ts";

test("a matcher reading requester fields raises the permission hint", () => {
  assert.equal(
    requestRuleReadsRequester(
      'deny if {\n  input.requester.username == "sample-member"\n}',
    ),
    true,
  );
  assert.equal(
    requestRuleReadsRequester(
      'approve if {\n  "auto_approve" in input["requester"].library_permissions\n}',
    ),
    true,
  );
});

test("requester mentioned only in a comment or another field does not", () => {
  assert.equal(
    requestRuleReadsRequester(
      '# input.requester is not read here\napprove if {\n  input.request.origin == "web"\n}',
    ),
    false,
  );
  assert.equal(
    requestRuleReadsRequester(
      'manual if {\n  input.request.origin == "api"\n}',
    ),
    false,
  );
});
