import assert from "node:assert/strict";
import test from "node:test";
import {
  findSchemaCompatibilityChanges,
  hasSchemaCompatibilityFailure,
} from "./check-graphql-schema-compat.mjs";

const BASE_SCHEMA = `
  type Query {
    title(id: ID!): Title
    search(term: String): [Title!]!
  }

  type Mutation {
    updateTitle(input: UpdateTitleInput!): TitlePayload!
  }

  type Title {
    id: ID!
    name: String!
    state: TitleState!
  }

  type TitlePayload {
    title: Title!
  }

  input UpdateTitleInput {
    id: ID!
    name: String
  }

  enum TitleState {
    ACTIVE
    ARCHIVED
  }
`;

function changesFor(newSchema) {
  return findSchemaCompatibilityChanges(BASE_SCHEMA, newSchema);
}

test("allows additive nullable fields", () => {
  const changes = changesFor(`
    type Query {
      title(id: ID!): Title
      search(term: String): [Title!]!
    }

    type Mutation {
      updateTitle(input: UpdateTitleInput!): TitlePayload!
    }

    type Title {
      id: ID!
      name: String!
      state: TitleState!
      overview: String
    }

    type TitlePayload {
      title: Title!
    }

    input UpdateTitleInput {
      id: ID!
      name: String
    }

    enum TitleState {
      ACTIVE
      ARCHIVED
    }
  `);

  assert.equal(hasSchemaCompatibilityFailure(changes), false);
});

test("allows the deprecated directive location that only affects schema authors", () => {
  const oldSchema = `
    directive @deprecated(reason: String = "No longer supported") on FIELD_DEFINITION | ARGUMENT_DEFINITION | INPUT_FIELD_DEFINITION | ENUM_VALUE | DIRECTIVE_DEFINITION

    type Query {
      title: String
    }
  `;
  const newSchema = `
    directive @deprecated(reason: String = "No longer supported") on FIELD_DEFINITION | ARGUMENT_DEFINITION | INPUT_FIELD_DEFINITION | ENUM_VALUE

    type Query {
      title: String
    }
  `;

  const changes = findSchemaCompatibilityChanges(oldSchema, newSchema);

  assert.equal(changes.breaking.length, 0);
  assert.equal(hasSchemaCompatibilityFailure(changes), false);
});

test("rejects removed fields", () => {
  const changes = changesFor(`
    type Query {
      title(id: ID!): Title
      search(term: String): [Title!]!
    }

    type Mutation {
      updateTitle(input: UpdateTitleInput!): TitlePayload!
    }

    type Title {
      id: ID!
      state: TitleState!
    }

    type TitlePayload {
      title: Title!
    }

    input UpdateTitleInput {
      id: ID!
      name: String
    }

    enum TitleState {
      ACTIVE
      ARCHIVED
    }
  `);

  assert.equal(hasSchemaCompatibilityFailure(changes), true);
  assert.ok(
    changes.breaking.some((change) => change.description.includes("name")),
  );
});

test("rejects changed field types", () => {
  const changes = changesFor(`
    type Query {
      title(id: ID!): Title
      search(term: String): [Title!]!
    }

    type Mutation {
      updateTitle(input: UpdateTitleInput!): TitlePayload!
    }

    type Title {
      id: ID!
      name: Int!
      state: TitleState!
    }

    type TitlePayload {
      title: Title!
    }

    input UpdateTitleInput {
      id: ID!
      name: String
    }

    enum TitleState {
      ACTIVE
      ARCHIVED
    }
  `);

  assert.equal(hasSchemaCompatibilityFailure(changes), true);
  assert.ok(
    changes.breaking.some((change) => change.description.includes("name")),
  );
});

test("rejects new required input fields and arguments", () => {
  const changes = changesFor(`
    type Query {
      title(id: ID!, libraryId: ID!): Title
      search(term: String): [Title!]!
    }

    type Mutation {
      updateTitle(input: UpdateTitleInput!): TitlePayload!
    }

    type Title {
      id: ID!
      name: String!
      state: TitleState!
    }

    type TitlePayload {
      title: Title!
    }

    input UpdateTitleInput {
      id: ID!
      name: String
      libraryId: ID!
    }

    enum TitleState {
      ACTIVE
      ARCHIVED
    }
  `);

  assert.equal(hasSchemaCompatibilityFailure(changes), true);
  assert.ok(
    changes.breaking.some((change) => change.description.includes("libraryId")),
  );
});

test("rejects enum value additions as dangerous", () => {
  const changes = changesFor(`
    type Query {
      title(id: ID!): Title
      search(term: String): [Title!]!
    }

    type Mutation {
      updateTitle(input: UpdateTitleInput!): TitlePayload!
    }

    type Title {
      id: ID!
      name: String!
      state: TitleState!
    }

    type TitlePayload {
      title: Title!
    }

    input UpdateTitleInput {
      id: ID!
      name: String
    }

    enum TitleState {
      ACTIVE
      ARCHIVED
      PAUSED
    }
  `);

  assert.equal(hasSchemaCompatibilityFailure(changes), true);
  assert.equal(changes.breaking.length, 0);
  assert.ok(
    changes.dangerous.some((change) => change.description.includes("PAUSED")),
  );
  assert.equal(
    hasSchemaCompatibilityFailure(changes, { allowDangerous: true }),
    false,
  );
});
