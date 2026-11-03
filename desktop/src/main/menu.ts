import { app, BrowserWindow, Menu, MenuItemConstructorOptions } from "electron";
import { allowWindowClose } from "../main";
// PHOTOVAULT: brand name for native menu labels.
import { appDisplayName } from "./photovault/brand";
import { setShouldHideDockIcon, shouldHideDockIcon } from "./services/store";

// PHOTOVAULT: strip - `mainWindow` was only used by the removed update check;
// the parameter stays so callers are unchanged.
// eslint-disable-next-line @typescript-eslint/no-unused-vars
export const createApplicationMenu = (_mainWindow: BrowserWindow) => {
    let hideDockIcon = shouldHideDockIcon();

    const macOSOnly = (options: MenuItemConstructorOptions[]) =>
        process.platform == "darwin" ? options : [];

    const toggleHideDockIcon = () => {
        setShouldHideDockIcon(!hideDockIcon);
        hideDockIcon = !hideDockIcon;
    };

    return Menu.buildFromTemplate([
        {
            // PHOTOVAULT: brand name.
            label: appDisplayName,
            submenu: [
                ...macOSOnly([
                    { label: `About ${appDisplayName}`, role: "about" },
                ]),
                // PHOTOVAULT: strip - no "Check for Updates..." (Ente's release
                // feed); updates come with the Photovault release you install.
                { type: "separator" },

                ...macOSOnly([
                    {
                        label: "Preferences",
                        submenu: [
                            {
                                label: "Hide Dock Icon",
                                type: "checkbox",
                                checked: hideDockIcon,
                                click: toggleHideDockIcon,
                            },
                        ],
                    },
                ]),

                { type: "separator" },
                ...macOSOnly([
                    // PHOTOVAULT: brand name.
                    { label: `Hide ${appDisplayName}`, role: "hide" },
                    { label: "Hide Others", role: "hideOthers" },
                    { type: "separator" },
                ]),
                { label: "Quit", role: "quit" },
            ],
        },
        {
            label: "Edit",
            submenu: [
                { label: "Undo", role: "undo" },
                { label: "Redo", role: "redo" },
                { type: "separator" },
                { label: "Cut", role: "cut" },
                { label: "Copy", role: "copy" },
                { label: "Paste", role: "paste" },
                { label: "Select All", role: "selectAll" },
                ...macOSOnly([
                    { type: "separator" },
                    {
                        label: "Speech",
                        submenu: [
                            { role: "startSpeaking", label: "Start Speaking" },
                            { role: "stopSpeaking", label: "Stop Speaking" },
                        ],
                    },
                ]),
            ],
        },
        {
            label: "View",
            submenu: [
                { label: "Reload", role: "reload" },
                { label: "Toggle Dev Tools", role: "toggleDevTools" },
                { type: "separator" },
                { label: "Toggle Full Screen", role: "togglefullscreen" },
            ],
        },
        {
            label: "Window",
            submenu: [
                { label: "Minimize", role: "minimize" },
                { label: "Zoom", role: "zoom" },
                { label: "Close", role: "close" },
                ...macOSOnly([
                    { type: "separator" },
                    { label: "Bring All to Front", role: "front" },
                    { type: "separator" },
                    // PHOTOVAULT: brand name.
                    { label: appDisplayName, role: "window" },
                ]),
            ],
        },
        // PHOTOVAULT: strip - Help menu (it only opened Ente's help site) removed.
    ]);
};

export const createTrayContextMenu = (mainWindow: BrowserWindow) => {
    const handleOpen = () => {
        mainWindow.show();
    };

    const handleClose = () => {
        allowWindowClose();
        app.quit();
    };

    return Menu.buildFromTemplate([
        // PHOTOVAULT: brand name.
        { label: `Open ${appDisplayName}`, click: handleOpen },
        { label: `Quit ${appDisplayName}`, click: handleClose },
    ]);
};
