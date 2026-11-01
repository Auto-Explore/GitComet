# auth-token.js

> GitHub API token authentication for browsers and Node.js

## Usage

<table>
<tbody valign=top align=left>
<tr><th>
Browsers
</th><td width=100%>

Load `@octokit/auth-token` directly from [esm.sh](https://esm.sh)

```html
<script type="module">
  import { createTokenAuth } from "https://esm.sh/@octokit/auth-token";
</script>
```

</td></tr>
<tr><th>
Node
</th><td>

Install with <code>npm install @octokit/auth-token</code>

```js
import { createTokenAuth } from "@octokit/auth-token";
```

</td></tr>
</tbody>
</table>

```js
const auth = createTokenAuth("ghp_PersonalAccessToken01245678900000000");
const authentication = await auth();
```

## Authentication object

<table width="100%">
  <thead align=left>
    <tr>
      <th width=150>
        name
      </th>
      <th width=70>
        type
      </th>
      <th>
        description
      </th>
    </tr>
  </thead>
  <tbody align=left valign=top>
    <tr>
      <th>
        <code>type</code>
      </th>
      <th>
        <code>string</code>
      </th>
      <td>
        <code>"token"</code>
      </td>
    </tr>
    <tr>
      <th>
        <code>token</code>
      </th>
      <th>
        <code>string</code>
      </th>
      <td>
        The provided token.
      </td>
    </tr>
    <tr>
      <th>
        <code>tokenType</code>
      </th>
      <th>
        <code>string</code>
      </th>
      <td>
        Can be either <code>"oauth"</code> for personal access tokens and OAuth tokens, <code>"installation"</code> for installation access tokens (includes <code>GITHUB_TOKEN</code> provided to GitHub Actions), <code>"app"</code> for a GitHub App JSON Web Token, or <code>"user-to-server"</code> for a user authentication token through an app installation.
      </td>
    </tr>
  </tbody>
</table>

## License

[MIT](LICENSE)
