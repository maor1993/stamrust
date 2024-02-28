# Stamrust - HTTP server demo 
an implmentation of CDC-NCM, HTTP, DHCP, all in rust!

inspired by: https://github.com/IntergatedCircuits/IPoverUSB


# Building
the project builds for the [stamdev board](https://www.tindie.com/products/maorm7/stamdev-l412/), which uses stm32l412 128K FLASH, 40K RAM.


this project uses the latest version of usb-device, which supports alt settings (requried for CDC-NCM)
due to this we need to "update" the versions used by the hal.

to do so- clone the following projects:
* [stm32-hal2](https://github.com/David-OConnor/stm32-hal)
* [stm32-usbd](https://github.com/stm32-rs/stm32-usbd)
* [usb-device](https://github.com/rust-embedded-community/usb-device)

set them in the folder structure
* THIS PROJECT
* stm32-hal
* stm32-usbd
* usb-device

change stm32-usbd usb-device dependecy to `usb-device = {path = "../usb-device"}`
change stm32-hal stm32-usbd dependecy to `stm32-usbd = { path="../stm32-usbd", optional = true }`

hopefully in the future this won't be needed, I will update the cargo.toml accordingly

# Use
one built and burnt, you should be able to connect to `192.168.69.1` on your webbrowser.

here you can control the RGB led on the board, and also see the number of program loops performed per second 

