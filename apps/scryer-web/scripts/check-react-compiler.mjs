import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformFileAsync } from "@babel/core";
import reactCompiler from "babel-plugin-react-compiler";

const appRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const tableModules = [
  {
    file: "components/views/media-content/compact-title-table.tsx",
    component: "CompactTitleTable",
  },
  {
    file: "components/views/media-content/title-table.tsx",
    component: "TitleTable",
  },
];

const failures = [];

for (const { file, component } of tableModules) {
  const events = [];
  await transformFileAsync(path.join(appRoot, file), {
    babelrc: false,
    configFile: false,
    parserOpts: {
      plugins: ["typescript", "jsx"],
    },
    plugins: [
      [
        reactCompiler,
        {
          compilationMode: "all",
          logger: {
            logEvent(filename, event) {
              events.push({ filename, event });
            },
          },
        },
      ],
    ],
  });

  const diagnostics = events.filter(({ event }) =>
    ["CompileError", "CompileDiagnostic", "PipelineError"].includes(event.kind),
  );
  if (diagnostics.length > 0) {
    failures.push(
      `${file} emitted compiler diagnostics:\n${diagnostics
        .map(
          ({ event }) =>
            `- ${event.kind}: ${JSON.stringify(event.detail ?? event.data)}`,
        )
        .join("\n")}`,
    );
  }

  const componentCompiled = events.some(
    ({ event }) =>
      event.kind === "CompileSuccess" && event.fnName === component,
  );
  if (!componentCompiled) {
    failures.push(`${file} did not compile ${component}.`);
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n\n"));
  process.exitCode = 1;
} else {
  console.log("React Compiler smoke check passed for title tables.");
}
