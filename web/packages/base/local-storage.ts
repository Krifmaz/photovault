import { savedServerStorageKey } from "ente-base/photovault/server"; // PHOTOVAULT: connect
import { nullToUndefined } from "ente-utils/transform";

// Preserve logs across logout.
// PHOTOVAULT: connect - also preserve the server chosen on the login page.
export const clearLocalStorage = () => {
    const existingLogs = nullToUndefined(localStorage.getItem("logs"));
    const savedServer = localStorage.getItem(savedServerStorageKey);
    localStorage.clear();
    if (existingLogs) {
        localStorage.setItem("logs", existingLogs);
    }
    if (savedServer) localStorage.setItem(savedServerStorageKey, savedServer);
};
