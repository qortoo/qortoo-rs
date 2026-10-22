import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import test from 'node:test';

const workflow = readFileSync(new URL('../workflows/benchmark-pr-dispatch.yml', import.meta.url), 'utf8');
const sdk = workflow.match(/github\.repository == 'qortoo\/(qortoo-(?:go|rs))'/)[1];
const component = sdk === 'qortoo-go' ? 'go' : 'rust';
const script = workflow.split('          script: |\n')[1].split('\n').map(line => line.slice(12)).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

for (const scenario of ['same-repository', 'fork-with-empty-associations', 'manual-fork', 'superseded', 'closed', 'wrong-base', 'wrong-base-repository', 'wrong-head-repository', 'wrong-branch', 'deleted-head-repository', 'missing-head-repository', 'missing-head-owner', 'missing-head-branch', 'failed-origin', 'wrong-workflow', 'invalid-sha', 'missing-token', 'invalid-input', 'dispatch-error']) {
  test(scenario, async () => {
    const sha = 'a'.repeat(40);
    const headOwner = scenario === 'same-repository' ? 'qortoo' : 'contributor';
    const headRepository = `${headOwner}/${sdk}`;
    // Real fork event runs may expose no PR associations in either API response.
    const origin = {event: 'pull_request', conclusion: 'success', path: '.github/workflows/benchmark-pr.yml', head_sha: sha,
      head_branch: 'test/candidate', head_repository: {full_name: headRepository, owner: {login: headOwner}}, pull_requests: []};
    const pr = {number: 7, state: 'open', draft: true, base: {ref: 'main', repo: {full_name: `qortoo/${sdk}`}},
      head: {sha, ref: origin.head_branch, repo: {full_name: headRepository}}};
    if (scenario === 'superseded') pr.head.sha = 'b'.repeat(40);
    if (scenario === 'closed') pr.state = 'closed';
    if (scenario === 'wrong-base') pr.base.ref = 'other';
    if (scenario === 'wrong-base-repository') pr.base.repo.full_name = 'unrelated/repo';
    if (scenario === 'wrong-head-repository') pr.head.repo.full_name = 'other/repo';
    if (scenario === 'wrong-branch') pr.head.ref = 'other';
    if (scenario === 'deleted-head-repository') pr.head.repo = null;
    if (scenario === 'missing-head-repository') origin.head_repository = null;
    if (scenario === 'missing-head-owner') origin.head_repository.owner = null;
    if (scenario === 'missing-head-branch') origin.head_branch = '';
    if (scenario === 'failed-origin') origin.conclusion = 'failure';
    if (scenario === 'wrong-workflow') origin.path = '.github/workflows/ci.yml';
    if (scenario === 'invalid-sha') origin.head_sha = 'invalid';
    const dispatched = [], failures = [];
    const summary = {addHeading() {return this;}, addRaw() {return this;}, addLink() {return this;}, addCodeBlock() {return this;}, async write() {}};
    const github = {rest: {pulls: {list() {}}, repos: {async listPullRequestsAssociatedWithCommit() {return {data: []};}}, actions: {
      async getWorkflowRun() {return {data: origin};},
      async createWorkflowDispatch(value) {dispatched.push(value); if (scenario === 'dispatch-error') throw Error('403 fixture'); return {data: {}};}
    }}, async paginate(method, args) {
      assert.equal(method, github.rest.pulls.list);
      assert.deepEqual(args, {owner: 'qortoo', repo: sdk, state: 'open', base: 'main', head: `${headOwner}:test/candidate`, per_page: 100});
      return [pr];
    }};
    const payload = scenario === 'manual-fork' ? {inputs: {source_run_id: '123', pr_number: '7'}} : {workflow_run: {id: 123}};
    if (scenario === 'invalid-input') payload.inputs = {source_run_id: 'invalid', pr_number: '7'};
    await new AsyncFunction('github', 'context', 'core', 'process', script)(github,
      {repo: {owner: 'qortoo', repo: sdk}, payload}, {setFailed: value => failures.push(value), summary},
      {env: {WORKFLOW_PAT: scenario === 'missing-token' ? '' : 'fixture'}});
    const sends = ['same-repository', 'fork-with-empty-associations', 'manual-fork', 'dispatch-error'].includes(scenario);
    assert.equal(dispatched.length, sends ? 1 : 0);
    const fails = ['missing-head-repository', 'missing-head-owner', 'missing-head-branch', 'failed-origin', 'wrong-workflow', 'invalid-sha', 'missing-token', 'invalid-input', 'dispatch-error'].includes(scenario);
    assert.equal(failures.length, fails ? 1 : 0);
    if (sends) assert.deepEqual(dispatched[0].inputs, {source_component: component, source_pr: '7', source_sha: sha, source_run_id: '123'});
  });
}
