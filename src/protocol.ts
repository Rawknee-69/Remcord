import path from "node:path";
import Url from "node:url";
import { app, net, protocol } from "electron";

protocol.registerSchemesAsPrivileged([
    {
        scheme: "remcord",
        privileges: {
            standard: true,
            secure: true,
            supportFetchAPI: true,
            corsEnabled: true,
            bypassCSP: true,
            stream: true,
        },
    },
]);

void app.whenReady().then(() => {
    // Remcord custom internal protocol
    protocol.handle("remcord", (req) => {
        if (req.url.startsWith("remcord://plugins/")) {
            const url = req.url.replace("remcord://plugins/", "").split("/");
            const filePath = path.join(import.meta.dirname, "plugins", `/${url[0]}/${url[1]}`);
            if (filePath.includes("..")) {
                return new Response("bad", {
                    status: 400,
                    headers: { "content-type": "text/html" },
                });
            }
            return net.fetch(Url.pathToFileURL(filePath).toString());
        } else if (req.url.startsWith("remcord://html/")) {
            const file = req.url.replace("remcord://html/", "");
            const filePath = path.join(import.meta.dirname, "html", `${file}`);
            if (filePath.includes("..")) {
                return new Response("bad", {
                    status: 400,
                    headers: { "content-type": "text/html" },
                });
            }
            return net.fetch(Url.pathToFileURL(filePath).toString());
        } else if (req.url.startsWith("remcord://js/")) {
            const file = req.url.replace("remcord://js/", "");
            const filePath = path.join(import.meta.dirname, "js", `${file}`);
            if (filePath.includes("..")) {
                return new Response("bad", {
                    status: 400,
                    headers: { "content-type": "text/html" },
                });
            }
            return net.fetch(Url.pathToFileURL(filePath).toString());
        } else if (req.url.startsWith("remcord://assets/")) {
            const file = req.url.replace("remcord://assets/", "");
            const filePath = path.join(import.meta.dirname, "assets", "app", `${file}`);
            if (filePath.includes("..")) {
                return new Response("bad", {
                    status: 400,
                    headers: { "content-type": "text/html" },
                });
            }
            return net.fetch(Url.pathToFileURL(filePath).toString());
        } else if (req.url.startsWith("remcord://css/")) {
            const file = req.url.replace("remcord://css/", "");
            const filePath = path.join(import.meta.dirname, "css", `${file}`);
            if (filePath.includes("..")) {
                return new Response("bad", {
                    status: 400,
                    headers: { "content-type": "text/html" },
                });
            }
            return net.fetch(Url.pathToFileURL(filePath).toString());
        } else if (req.url.startsWith("remcord://local/")) {
            const file = req.url.replace("remcord://local/", "");
            const userDataPath = path.join(app.getPath("userData"), "userAssets");
            const filePath = path.normalize(path.join(userDataPath, `${file}`));
            if (!filePath.startsWith(userDataPath)) {
                return new Response("bad", {
                    status: 400,
                    headers: { "content-type": "text/html" },
                });
            }
            return net.fetch(Url.pathToFileURL(filePath).toString());
        }
        return new Response("bad", {
            status: 400,
            headers: { "content-type": "text/html" },
        });
    });
});
