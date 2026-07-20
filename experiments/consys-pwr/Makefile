# Out-of-tree build for consys_pwr.ko
#
# Requires:
#   - armv7 hardfloat cross compiler
#   - prepared 3.18 kernel tree matching device (modules_prepare)
#   - device kernel with CONFIG_MODULES=y  (stock DigiLand = n)
#
# Example:
#   export CROSS_COMPILE=$HOME/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-
#   export ARCH=arm
#   export KDIR=$REPO/reference/upstream/kernel_amazon_mt8127-common
#   make

obj-m += consys_pwr.o

KDIR   ?= /lib/modules/$(shell uname -r)/build
ARCH   ?= arm
CROSS_COMPILE ?= armv7l-linux-musleabihf-

# Force vermagic-ish flags for ARMv7 Cortex-A7 (override as needed)
EXTRA_CFLAGS += -Wall -march=armv7-a

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) ARCH=$(ARCH) CROSS_COMPILE=$(CROSS_COMPILE) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) ARCH=$(ARCH) CROSS_COMPILE=$(CROSS_COMPILE) clean
	rm -f Module.symvers modules.order

.PHONY: all clean
