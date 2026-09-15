import { execFileSync } from 'node:child_process';
import conventionalCommits from 'conventional-changelog-conventionalcommits';
import { generateNotes as defaultGenerateNotes } from '@semantic-release/release-notes-generator';

const pullRequestSuffix = /\s+\(#(\d+)\)\s*$/;

function pullRequestNumber(commit) {
  return (commit.header || commit.message || '').match(pullRequestSuffix)?.[1] || null;
}

function withoutPullRequest(value, number) {
  return typeof value === 'string' ? value.replace(new RegExp(`\\s+\\(#${number}\\)\\s*$`), '') : value;
}

function authorKey(author) {
  if (!author || typeof author !== 'object') return null;
  const value = author.email || author.name;
  return typeof value === 'string' && value.trim() ? value.trim().toLowerCase() : null;
}

function githubLogin(author) {
  const email = author?.email?.trim();
  return email?.match(/^(?:\d+\+)?([^@]+)@users\.noreply\.github\.com$/i)?.[1] || null;
}

function authorLabel(author, context) {
  const login = githubLogin(author);
  if (login) return `[@${login}](${context.host}/${login})`;
  const name = author?.name?.trim();
  return name?.replace(/[\r\n]+/g, ' ') || null;
}

// ponytail: compare Git author identities; use GitHub contribution metadata if aliases or non-code contributions matter.
function previousAuthors(ref) {
  if (!ref) return new Set();
  try {
    return new Set(
      execFileSync('git', ['log', ref, '--format=%an%x09%ae'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
        .split('\n')
        .map(line => {
          const [name, email] = line.split('\t');
          return authorKey({ name, email });
        })
        .filter(Boolean),
    );
  } catch {
    return new Set();
  }
}

async function generateAnnotatedNotes(pluginConfig, context, previousRef) {
  const preset = await conventionalCommits(pluginConfig.presetConfig);
  const baseTransform = preset.writer.transform;
  const previous = previousAuthors(previousRef);
  const writerOpts = {
    ...pluginConfig.writerOpts,
    transform(commit, writerContext) {
      const pullRequest = pullRequestNumber(commit);
      const parsedCommit = pullRequest
        ? {
            ...commit,
            header: withoutPullRequest(commit.header, pullRequest),
            subject: withoutPullRequest(commit.subject, pullRequest),
            references: commit.references.filter(reference => String(reference.issue) !== pullRequest),
          }
        : commit;
      const transformed = baseTransform(parsedCommit, writerContext);
      if (!transformed) return transformed;
      return {
        ...transformed,
        author: authorLabel(commit.author, writerContext),
        authorKey: authorKey(commit.author),
        pullRequest,
        ...(pullRequest && { header: parsedCommit.header }),
      };
    },
    commitPartial: preset.writer.commitPartial.replace(
      '{{~!-- commit link --}}',
      '{{~#if author}} by {{author}}{{~/if}}{{~#if pullRequest}} in [#{{pullRequest}}]({{~@root.host}}/{{~@root.owner}}/{{~@root.repository}}/pull/{{pullRequest}}){{~/if}}\n\n{{~!-- commit link --}}',
    ),
    mainTemplate: `{{> header}}
{{#if noteGroups}}
{{#each noteGroups}}

### ⚠ {{title}}

{{#each notes}}
* {{#if commit.scope}}**{{commit.scope}}:** {{/if}}{{text}}
{{/each}}
{{/each}}
{{/if}}
{{#each commitGroups}}

{{#if title}}
### {{title}}

{{/if}}
{{#each commits}}
{{> commit root=@root}}
{{/each}}
{{/each}}
{{#if newContributors}}

## New Contributors

{{#each newContributors}}
* {{text}}
{{/each}}
{{/if}}
`,
    finalizeContext(templateContext, _options, commits) {
      const newContributors = [];
      const seen = new Set();
      for (const commit of commits) {
        if (!commit.pullRequest || !commit.authorKey || previous.has(commit.authorKey) || seen.has(commit.authorKey)) continue;
        if (commit.author?.includes('[bot]')) continue;
        seen.add(commit.authorKey);
        const url = `${templateContext.host}/${templateContext.owner}/${templateContext.repository}/pull/${commit.pullRequest}`;
        newContributors.push({ text: `${commit.author} made their first contribution in [#${commit.pullRequest}](${url})` });
      }
      return { ...templateContext, newContributors };
    },
  };
  return defaultGenerateNotes({ ...pluginConfig, writerOpts }, context);
}

export function getLatestReleaseTag(ref = 'HEAD') {
  try {
    execFileSync('git', ['describe', '--tags', '--match', 'v[0-9]*', '--exact-match', ref], { stdio: 'ignore' });
    return execFileSync('git', ['describe', '--tags', '--match', 'v[0-9]*', '--abbrev=0', `${ref}~1`], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    try {
      return execFileSync('git', ['describe', '--tags', '--match', 'v[0-9]*', '--abbrev=0', ref], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
    } catch {
      return null;
    }
  }
}

export async function generateReleaseNotes(channel, pluginConfig, context) {
  if (channel === 'production') {
    return generateAnnotatedNotes(pluginConfig, context, context.lastRelease?.gitTag || context.lastRelease?.gitHead);
  }

  const targetRef = context.nextRelease?.gitHead || 'HEAD';
  const latestTag = getLatestReleaseTag(targetRef);

  if (!latestTag) {
    return generateAnnotatedNotes(pluginConfig, context, context.lastRelease?.gitTag || context.lastRelease?.gitHead);
  }

  let nightlyHashes;
  try {
    nightlyHashes = new Set(
      execFileSync('git', ['rev-list', `${latestTag}..${targetRef}`], { encoding: 'utf8' })
        .split('\n')
        .map(h => h.trim())
        .filter(Boolean)
    );
  } catch {
    return generateAnnotatedNotes(pluginConfig, context, latestTag);
  }

  const nightlyCommits = (context.commits || []).filter(c => nightlyHashes.has(c.hash));
  const date = new Date().toISOString().slice(0, 10).replace(/-/g, '');
  const runNumber = process.env.GITHUB_RUN_NUMBER;
  const nightlyVersion = context.nextRelease?.version?.includes('nightly')
    ? context.nextRelease.version
    : (runNumber ? `${context.nextRelease.version}-nightly.${date}.${runNumber}` : `${context.nextRelease.version}-nightly`);

  return generateAnnotatedNotes(pluginConfig, {
    ...context,
    commits: nightlyCommits,
    lastRelease: {
      gitTag: latestTag,
      gitHead: latestTag,
      version: latestTag.replace(/^v/, ''),
    },
    nextRelease: {
      ...context.nextRelease,
      version: nightlyVersion,
      gitTag: `v${nightlyVersion}`,
    },
  }, latestTag);
}
