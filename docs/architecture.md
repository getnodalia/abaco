# Architecture

```text
nodalia-units  nodalia-numerics      shared engines, no app dependencies
        \            /
    abaco-expr    abaco-script           languages: the units-aware expression language;
        |              |                 the MATLAB-class matrix language (scripts, functions)
        |              |
    abaco-doc          |                 document, dependency graph, .abaco format
        \\            /
   abaco-cli   abaco-app                 front ends
```

Rules:

* Dependencies point down. Engines never use egui, eframe or winit, and none blocks on I/O it was not
  asked to do.
* Anything a headless client (the CLI, a script, Circuito, another Nodalia app) needs lives in an
  engine crate.
* `nodalia-units` and `nodalia-numerics` are shared engines. They stay in this repository until a
  third consumer needs them; then they move to a repository of their own so nobody depends on an app
  to get a library. Their names avoid the existing `nodalia-math`, `nodalia-sketch` and
  `nodalia-mesh` of Foundation.
* A failed cell changes nothing in the environment (all-or-nothing), so the document is always in a
  state a person can reason about.
