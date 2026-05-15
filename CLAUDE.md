# Understanding this project

* The original brief for this project is in @coding-guidelines/brief.md.
* The @README.md has instructions for how to use the project and explains the architecture.
* When writing a new app, follow @coding-guidelines/app_guide.md exactly.

## Integration tests

* Cucumber integration tests for all Corre apps live in @apps/e2e.
* Documentation for cucumber-rs is available at: https://cucumber-rs.github.io/cucumber/current/
* Each app has a dedicated module `apps/e2e/src/{appname}` where the world, step, and test clause definitions are stored.
* Each app has a dedicated test runner in `apps/e2e/src/e2e-{appname}.rs` that bootstraps the cucumber tests.
* Each app has a dedicated features folder in `apps/e2e/tests/features/{appname}`. The feature tests are stored in this folder.

When you run the integration tests:

* Make a note of failing tests. Decide if it would be valuable to add a simplified unit test to the code to a) fix the test on a more 
  granular level and b) guard against regression in the future. If so, DO NOT write the test immediately. Instead, save instructions to 
  an agent to write the test and save it under `plans/todos/{unittest}-{slug}.md` in the following format:

```md
  status: todo
  instructions: Write a regression unit test for {app/module}....
 
  example input 1:
  {example input}
  
  example output 1:
  {expected output}
  
  ..repeat for other cases
``` 

## Security considerations

Always follow the guidelines in @coding-guidelines/security-best-practices.md. Keep security top of mind when writing code.
Every time you write code, compare it against the security best practices and ask yourself: Does this code follow the security best 
practices.

If you are asked to implement a feature that violates security best-practice, STOP! Point out the violation, why it is a bad idea, and 
suggest an alternative approach or suggest dropping the feature completely.