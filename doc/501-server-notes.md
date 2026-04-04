


Set the shrmpl-cicd as a service

vi /srv/shrmpl/etc/shrmpl-cicd.service
```
# ln -s /srv/shrmpl/etc/shrmpl-cicd.service /etc/systemd/system/shrmpl-cicd.service

[Unit]
Description=shrmpl CICD webhook server
After=network.target

[Service]
Type=simple
User=shrmpl
ExecStart=/srv/shrmpl/libexec/shrmpl-cicd-srv /srv/shrmpl/etc/cicd-srv.env
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## create a symlink to keep the real config in the shmpl directory
```
ln -s /srv/shrmpl/etc/shrmpl-cicd.service /etc/systemd/system/shrmpl-cicd.servic

systemctl daemon-reload
systemctl enable shrmpl-cicd
systemctl start shrmpl-cicd
# systemctl stop shrmpl-cicd
# systemctl restart shrmpl-cicd
```

## as root user setup env
```
useradd -r -s /bin/bash -d /build-stg shrmpl-cicd
mkdir -p /build-stg/.ssh
chmod -R 777 /build-stg
chown shrmpl-cicd:shrmpl-cicd /build-stg/.ssh

# add shrmpl-cicd to the docker group on the server
usermod -aG docker shrmpl-cicd
systemctl restart shrmpl-cicd
systemctl restart docker

```

## get a shrmpl-cicd user shell, setup env
```
su - shrmpl-cicd
cd /build-stg/.ssh
#note: must be rsa
ssh-keygen -t rsa -C "shrmpl-cicd@atticus-cicd-001" -f /srv/shrmpl/.ssh/azdo_shrmpl -N ""
# you have to do the initial clone as the shrmplcicd user
git clone git@ssh.dev.azure.com:v3/Quantous/Epsilon/atticus-onesheet
```

## create the config file
vi /build-stg/.ssh/config
```
Host ssh.dev.azure.com
    IdentityFile /srv/shrmpl/.ssh/azdo_shrmpl
    IdentitiesOnly yes
```


## Creating the other services

```
useradd --system --no-create-home --shell /usr/sbin/nologin shrmpl
```

```
vi /srv/shrmpl/etc/shrmpl-log.service
```

```

# ln -s /srv/shrmpl/etc/shrmpl-log-central.service /etc/systemd/system/shrmpl-log-central.service

[Unit]
Description=shrmpl log reciever
After=network.target

[Service]
Type=simple
User=shrmpl
ExecStart=/srv/shrmpl/libexec/shrmpl-log-srv /srv/shrmpl/etc/shrmpl-log-central.env
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```
systemctl daemon-reload
systemctl start shrmpl-log-central
systemctl enable shrmpl-log-central
journalctl -u shrmpl-log-central -f
```
```
vi /srv/shrmpl/etc/shrmpl-kv-srv-central.env
```

```
# ln -s /srv/shrmpl/etc/shrmpl-kv-central.service /etc/systemd/system/shrmpl-kv-central.service

[Unit]
Description=shrmpl kv reciever
After=network.target

[Service]
Type=simple
User=shrmpl
ExecStart=/srv/shrmpl/libexec/shrmpl-kv-srv /srv/shrmpl/etc/shrmpl-kv-srv-central.env
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```
systemctl daemon-reload
systemctl start shrmpl-kv-central
systemctl enable shrmpl-kv-central
journalctl -u shrmpl-kv-central -f

```
