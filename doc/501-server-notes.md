


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
