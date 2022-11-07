/* eslint-disable @typescript-eslint/explicit-module-boundary-types */

import React from "react";
import type {URL} from "url";

import type { OidcProvider } from "./main.js";

const icons: Record<string, string> = {
	google: "data:image/svg+xml;base64,PD94bWwgdmVyc2lvbj0iMS4wIiBlbmNvZGluZz0iVVRGLTgiIHN0YW5kYWxvbmU9Im5vIj8+Cjxzdmcgd2lkdGg9IjE4cHgiIGhlaWdodD0iMThweCIgdmlld0JveD0iMCAwIDE4IDE4IiB2ZXJzaW9uPSIxLjEiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgeG1sbnM6eGxpbms9Imh0dHA6Ly93d3cudzMub3JnLzE5OTkveGxpbmsiPgogICAgPCEtLSBHZW5lcmF0b3I6IFNrZXRjaCA0Ni4yICg0NDQ5NikgLSBodHRwOi8vd3d3LmJvaGVtaWFuY29kaW5nLmNvbS9za2V0Y2ggLS0+CiAgICA8dGl0bGU+bG9nb19nb29nbGVnXzQ4ZHA8L3RpdGxlPgogICAgPGRlc2M+Q3JlYXRlZCB3aXRoIFNrZXRjaC48L2Rlc2M+CiAgICA8ZGVmcz48L2RlZnM+CiAgICA8ZyBpZD0iUGFnZS0xIiBzdHJva2U9Im5vbmUiIHN0cm9rZS13aWR0aD0iMSIgZmlsbD0ibm9uZSIgZmlsbC1ydWxlPSJldmVub2RkIj4KICAgICAgICA8ZyBpZD0ibG9nb19nb29nbGVnXzQ4ZHAiPgogICAgICAgICAgICA8cGF0aCBkPSJNMTcuNjQsOS4yMDQ1NDU0NSBDMTcuNjQsOC41NjYzNjM2NCAxNy41ODI3MjczLDcuOTUyNzI3MjcgMTcuNDc2MzYzNiw3LjM2MzYzNjM2IEw5LDcuMzYzNjM2MzYgTDksMTAuODQ1IEwxMy44NDM2MzY0LDEwLjg0NSBDMTMuNjM1LDExLjk3IDEzLjAwMDkwOTEsMTIuOTIzMTgxOCAxMi4wNDc3MjczLDEzLjU2MTM2MzYgTDEyLjA0NzcyNzMsMTUuODE5NTQ1NSBMMTQuOTU2MzYzNiwxNS44MTk1NDU1IEMxNi42NTgxODE4LDE0LjI1MjcyNzMgMTcuNjQsMTEuOTQ1NDU0NSAxNy42NCw5LjIwNDU0NTQ1IEwxNy42NCw5LjIwNDU0NTQ1IFoiIGlkPSJTaGFwZSIgZmlsbD0iIzQyODVGNCIgZmlsbC1ydWxlPSJub256ZXJvIj48L3BhdGg+CiAgICAgICAgICAgIDxwYXRoIGQ9Ik05LDE4IEMxMS40MywxOCAxMy40NjcyNzI3LDE3LjE5NDA5MDkgMTQuOTU2MzYzNiwxNS44MTk1NDU1IEwxMi4wNDc3MjczLDEzLjU2MTM2MzYgQzExLjI0MTgxODIsMTQuMTAxMzYzNiAxMC4yMTA5MDkxLDE0LjQyMDQ1NDUgOSwxNC40MjA0NTQ1IEM2LjY1NTkwOTA5LDE0LjQyMDQ1NDUgNC42NzE4MTgxOCwxMi44MzcyNzI3IDMuOTY0MDkwOTEsMTAuNzEgTDAuOTU3MjcyNzI3LDEwLjcxIEwwLjk1NzI3MjcyNywxMy4wNDE4MTgyIEMyLjQzODE4MTgyLDE1Ljk4MzE4MTggNS40ODE4MTgxOCwxOCA5LDE4IEw5LDE4IFoiIGlkPSJTaGFwZSIgZmlsbD0iIzM0QTg1MyIgZmlsbC1ydWxlPSJub256ZXJvIj48L3BhdGg+CiAgICAgICAgICAgIDxwYXRoIGQ9Ik0zLjk2NDA5MDkxLDEwLjcxIEMzLjc4NDA5MDkxLDEwLjE3IDMuNjgxODE4MTgsOS41OTMxODE4MiAzLjY4MTgxODE4LDkgQzMuNjgxODE4MTgsOC40MDY4MTgxOCAzLjc4NDA5MDkxLDcuODMgMy45NjQwOTA5MSw3LjI5IEwzLjk2NDA5MDkxLDQuOTU4MTgxODIgTDAuOTU3MjcyNzI3LDQuOTU4MTgxODIgQzAuMzQ3NzI3MjczLDYuMTczMTgxODIgMCw3LjU0NzcyNzI3IDAsOSBDMCwxMC40NTIyNzI3IDAuMzQ3NzI3MjczLDExLjgyNjgxODIgMC45NTcyNzI3MjcsMTMuMDQxODE4MiBMMy45NjQwOTA5MSwxMC43MSBMMy45NjQwOTA5MSwxMC43MSBaIiBpZD0iU2hhcGUiIGZpbGw9IiNGQkJDMDUiIGZpbGwtcnVsZT0ibm9uemVybyI+PC9wYXRoPgogICAgICAgICAgICA8cGF0aCBkPSJNOSwzLjU3OTU0NTQ1IEMxMC4zMjEzNjM2LDMuNTc5NTQ1NDUgMTEuNTA3NzI3Myw0LjAzMzYzNjM2IDEyLjQ0MDQ1NDUsNC45MjU0NTQ1NSBMMTUuMDIxODE4MiwyLjM0NDA5MDkxIEMxMy40NjMxODE4LDAuODkxODE4MTgyIDExLjQyNTkwOTEsMCA5LDAgQzUuNDgxODE4MTgsMCAyLjQzODE4MTgyLDIuMDE2ODE4MTggMC45NTcyNzI3MjcsNC45NTgxODE4MiBMMy45NjQwOTA5MSw3LjI5IEM0LjY3MTgxODE4LDUuMTYyNzI3MjcgNi42NTU5MDkwOSwzLjU3OTU0NTQ1IDksMy41Nzk1NDU0NSBMOSwzLjU3OTU0NTQ1IFoiIGlkPSJTaGFwZSIgZmlsbD0iI0VBNDMzNSIgZmlsbC1ydWxlPSJub256ZXJvIj48L3BhdGg+CiAgICAgICAgICAgIDxwb2x5Z29uIGlkPSJTaGFwZSIgcG9pbnRzPSIwIDAgMTggMCAxOCAxOCAwIDE4Ij48L3BvbHlnb24+CiAgICAgICAgPC9nPgogICAgPC9nPgo8L3N2Zz4=",
	microsoft: "data:image/svg+xml;base64,PD94bWwgdmVyc2lvbj0iMS4wIiBlbmNvZGluZz0iVVRGLTgiIHN0YW5kYWxvbmU9Im5vIj8+CjwhRE9DVFlQRSBzdmcgUFVCTElDICItLy9XM0MvL0RURCBTVkcgMS4xLy9FTiIgImh0dHA6Ly93d3cudzMub3JnL0dyYXBoaWNzL1NWRy8xLjEvRFREL3N2ZzExLmR0ZCI+CjxzdmcgeG1sbnM9Imh0dHA6Ly93d3cudzMub3JnLzIwMDAvc3ZnIiB2ZXJzaW9uPSIxLjEiIGhlaWdodD0iNDM5IiB3aWR0aD0iNDM5Ij4KPHJlY3QgaGVpZ2h0PSI0MzkiIHdpZHRoPSI0MzkiIGZpbGw9IiNmM2YzZjMiLz4KPHJlY3QgaGVpZ2h0PSIxOTQiIHdpZHRoPSIxOTQiIHg9IjE3IiAgeT0iMTciICBmaWxsPSIjRjM1MzI1Ii8+CjxyZWN0IGhlaWdodD0iMTk0IiB3aWR0aD0iMTk0IiB4PSIyMjgiIHk9IjE3IiAgZmlsbD0iIzgxQkMwNiIvPgo8cmVjdCBoZWlnaHQ9IjE5NCIgd2lkdGg9IjE5NCIgeD0iMTciICB5PSIyMjgiIGZpbGw9IiMwNUE2RjAiLz4KPHJlY3QgaGVpZ2h0PSIxOTQiIHdpZHRoPSIxOTQiIHg9IjIyOCIgeT0iMjI4IiBmaWxsPSIjRkZCQTA4Ii8+Cjwvc3ZnPg==",
};

export const loginPage = (providers: { provider: OidcProvider, url: string }[]) => <html>
	<head>
		<title>Login</title>
		<style>{`
			html {
				background-image: linear-gradient(0deg, rgb(24 100 218 / 10%), white);
				min-height: 100vh;
				min-width: 100vw;
			}
			body {
				display: flex;
				flex-wrap: wrap;
				align-items: center;
				align-content: center;
				justify-content: center;
				height: 250px;
				width: 400px;
				margin: auto;
				margin-top: calc(50vh - 150px);
				border-radius: 5px;
				box-shadow: 0 6px 20px 0px rgb(9 14 37 / 10%);
				background: white;
			}
			a {
				padding: 6px 24px !important;
				margin: 12px 0px !important;
				text-decoration: none;
				width: 230px !important;
				font-size: 14px;
				font-weight: bold !important;
				color: rgb(31 57 97 / 66%) !important;
				transition: all .2s ease-in-out;
				border: 1px solid white !important;
				font-family: Arial, sans-serif !important;
				text-align: center;
				box-shadow: 0 2px 8px 0px rgb(9 14 37 / 15%)!important;
				border-radius: 3px;
				line-height: 30px;
				height: 30px;
				flex-flow: column wrap;
				justify-content: center;
				align-items: start;
				display: flex;
				background: #fff;
			}
			a:hover {
				background: rgb(31 57 97 / 5%) !important;
				border: 1px solid rgb(24 100 218 / 50%)!important;
				box-shadow: 0 4px 15px 0px rgb(9 14 37 / 15%)!important;
			}
		`}</style>
	</head>
	<body>
		{providers.map(({ provider, url }) => <a key={provider.id} href={url}><span><img src={icons[provider.id]} height="20" /></span> Login with {provider.desc}</a>)}
	</body>
</html>;

export const notAuthorizedPage = (email: string, url: URL) => <html>
	<head>
		<title>Not Authorized</title>
	</head>
	<body>
		<p>Sorry, {email} is not allowed to access this page on {url.origin}</p>
		<p><a href={url.toString()}>Back</a></p>
	</body>
</html>;
